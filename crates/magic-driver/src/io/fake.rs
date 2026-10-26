use std::cell::RefCell;
use std::collections::VecDeque;
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use md_core::Button;
use md_core::synth::Frame;
use nix::errno::Errno;

use super::{
    DeviceDirectory, DeviceError, DeviceOpener, EvdevEvent, RawSource, StockDevice, VirtualPointer,
    VirtualTouchpad,
};
use crate::discovery::DiscoveryEvent;

#[derive(Debug, Clone, PartialEq)]
pub enum Call {
    OpenRaw(PathBuf),
    CloseRaw,
    OpenStock(PathBuf),
    CloseStock,
    Grab,
    Ungrab,
    CreatePointer,
    DestroyPointer,
    Motion { dx: i32, dy: i32 },
    Button { button: Button, pressed: bool },
    CreateTouchpad,
    DestroyTouchpad,
    Frame(Frame),
}

#[derive(Debug, Default)]
pub struct World {
    pub calls: Vec<Call>,
    pub open_raw_errors: VecDeque<Errno>,
    pub open_stock_errors: VecDeque<Errno>,
    pub create_pointer_errors: VecDeque<Errno>,
    pub create_touchpad_errors: VecDeque<Errno>,
    pub grab_errors: VecDeque<Errno>,
    pub ungrab_error: Option<Errno>,
    pub raw_reports: VecDeque<Result<Vec<u8>, Errno>>,
    pub stock_events: VecDeque<Result<Vec<EvdevEvent>, Errno>>,
    pub pointer_write_error: Option<Errno>,
    pub enumerated: Vec<DiscoveryEvent>,
    pub udev_changes: VecDeque<DiscoveryEvent>,
    pub grabbed: bool,
}

pub type SharedWorld = Rc<RefCell<World>>;

pub fn world() -> SharedWorld {
    Rc::new(RefCell::new(World::default()))
}

fn log(world: &SharedWorld, call: Call) {
    world.borrow_mut().calls.push(call);
}

fn fail(queue: &mut VecDeque<Errno>) -> Result<(), DeviceError> {
    match queue.pop_front() {
        Some(errno) => Err(DeviceError::from(errno)),
        None => Ok(()),
    }
}

#[derive(Debug, Clone)]
pub struct FakeOpener {
    pub world: SharedWorld,
}

impl DeviceOpener for FakeOpener {
    type Raw = FakeRaw;
    type Stock = FakeStock;
    type Pointer = FakePointer;
    type Touchpad = FakeTouchpad;

    fn open_raw(&mut self, path: &Path) -> Result<FakeRaw, DeviceError> {
        fail(&mut self.world.borrow_mut().open_raw_errors)?;
        log(&self.world, Call::OpenRaw(path.to_path_buf()));
        Ok(FakeRaw {
            world: self.world.clone(),
        })
    }

    fn open_stock(&mut self, path: &Path) -> Result<FakeStock, DeviceError> {
        fail(&mut self.world.borrow_mut().open_stock_errors)?;
        log(&self.world, Call::OpenStock(path.to_path_buf()));
        Ok(FakeStock {
            world: self.world.clone(),
        })
    }

    fn create_pointer(&mut self) -> Result<FakePointer, DeviceError> {
        fail(&mut self.world.borrow_mut().create_pointer_errors)?;
        log(&self.world, Call::CreatePointer);
        Ok(FakePointer {
            world: self.world.clone(),
        })
    }

    fn create_touchpad(&mut self) -> Result<FakeTouchpad, DeviceError> {
        fail(&mut self.world.borrow_mut().create_touchpad_errors)?;
        log(&self.world, Call::CreateTouchpad);
        Ok(FakeTouchpad {
            world: self.world.clone(),
        })
    }
}

#[derive(Debug)]
pub struct FakeRaw {
    world: SharedWorld,
}

impl RawSource for FakeRaw {
    fn read_report(&mut self, buf: &mut [u8]) -> Result<Option<usize>, DeviceError> {
        match self.world.borrow_mut().raw_reports.pop_front() {
            None => Ok(None),
            Some(Ok(bytes)) => {
                buf[..bytes.len()].copy_from_slice(&bytes);
                Ok(Some(bytes.len()))
            }
            Some(Err(errno)) => Err(DeviceError::from(errno)),
        }
    }

    fn poll_fd(&self) -> Option<BorrowedFd<'_>> {
        None
    }
}

impl Drop for FakeRaw {
    fn drop(&mut self) {
        log(&self.world, Call::CloseRaw);
    }
}

#[derive(Debug)]
pub struct FakeStock {
    world: SharedWorld,
}

impl StockDevice for FakeStock {
    fn grab(&mut self) -> Result<(), DeviceError> {
        fail(&mut self.world.borrow_mut().grab_errors)?;
        self.world.borrow_mut().grabbed = true;
        log(&self.world, Call::Grab);
        Ok(())
    }

    fn ungrab(&mut self) -> Result<(), DeviceError> {
        self.world.borrow_mut().grabbed = false;
        log(&self.world, Call::Ungrab);
        match self.world.borrow_mut().ungrab_error.take() {
            Some(errno) => Err(DeviceError::from(errno)),
            None => Ok(()),
        }
    }

    fn drain(&mut self, out: &mut Vec<EvdevEvent>) -> Result<(), DeviceError> {
        loop {
            match self.world.borrow_mut().stock_events.pop_front() {
                None => return Ok(()),
                Some(Ok(events)) => out.extend(events),
                Some(Err(errno)) => return Err(DeviceError::from(errno)),
            }
        }
    }

    fn poll_fd(&self) -> Option<BorrowedFd<'_>> {
        None
    }
}

impl Drop for FakeStock {
    fn drop(&mut self) {
        self.world.borrow_mut().grabbed = false;
        log(&self.world, Call::CloseStock);
    }
}

#[derive(Debug)]
pub struct FakePointer {
    world: SharedWorld,
}

impl FakePointer {
    fn check_write(&self) -> Result<(), DeviceError> {
        match self.world.borrow_mut().pointer_write_error.take() {
            Some(errno) => Err(DeviceError::from(errno)),
            None => Ok(()),
        }
    }
}

impl VirtualPointer for FakePointer {
    fn motion(&mut self, dx: i32, dy: i32) -> Result<(), DeviceError> {
        self.check_write()?;
        log(&self.world, Call::Motion { dx, dy });
        Ok(())
    }

    fn button(&mut self, button: Button, pressed: bool) -> Result<(), DeviceError> {
        self.check_write()?;
        log(&self.world, Call::Button { button, pressed });
        Ok(())
    }
}

impl Drop for FakePointer {
    fn drop(&mut self) {
        log(&self.world, Call::DestroyPointer);
    }
}

#[derive(Debug)]
pub struct FakeTouchpad {
    world: SharedWorld,
}

impl VirtualTouchpad for FakeTouchpad {
    fn frame(&mut self, frame: &Frame) -> Result<(), DeviceError> {
        log(&self.world, Call::Frame(*frame));
        Ok(())
    }
}

impl Drop for FakeTouchpad {
    fn drop(&mut self) {
        log(&self.world, Call::DestroyTouchpad);
    }
}

#[derive(Debug, Clone)]
pub struct FakeDirectory {
    pub world: SharedWorld,
}

impl DeviceDirectory for FakeDirectory {
    fn enumerate(&mut self, out: &mut Vec<DiscoveryEvent>) -> Result<(), DeviceError> {
        out.extend(self.world.borrow().enumerated.iter().cloned());
        Ok(())
    }

    fn changes(&mut self, out: &mut Vec<DiscoveryEvent>) -> Result<(), DeviceError> {
        out.extend(self.world.borrow_mut().udev_changes.drain(..));
        Ok(())
    }

    fn poll_fd(&self) -> Option<BorrowedFd<'_>> {
        None
    }
}

use std::path::Path;

use md_core::config::Config;
use md_core::{Micros, Mode};

use super::{ReleaseReason, Session, SessionFds, SessionObserver, SessionState};
use crate::discovery::{DiscoveryEvent, MouseNodes};
use crate::io::DeviceOpener;

pub struct SessionManager<O: DeviceOpener> {
    opener: O,
    mode: Mode,
    config: Config,
    known: Vec<MouseNodes>,
    session: Option<Session<O>>,
}

impl<O: DeviceOpener> std::fmt::Debug for SessionManager<O> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionManager")
            .field("mode", &self.mode)
            .field("known", &self.known)
            .field("session", &self.session)
            .finish_non_exhaustive()
    }
}

impl<O: DeviceOpener> SessionManager<O> {
    pub fn new(opener: O, mode: Mode, config: Config) -> SessionManager<O> {
        SessionManager {
            opener,
            mode,
            config,
            known: Vec::new(),
            session: None,
        }
    }

    #[cfg(test)]
    pub fn session(&self) -> Option<&Session<O>> {
        self.session.as_ref()
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn apply_config(
        &mut self,
        now: Micros,
        config: Config,
        observer: &mut dyn SessionObserver,
    ) {
        if let Some(session) = self.session.as_mut() {
            session.apply_config(now, &config, observer);
        }
        self.config = config;
    }

    pub fn status_line(&self) -> String {
        self.session.as_ref().map_or_else(
            || "waiting: no Magic Mouse 2 connected".to_owned(),
            Session::status_line,
        )
    }

    pub fn known(&self) -> &[MouseNodes] {
        &self.known
    }

    pub fn fds(&self) -> Option<SessionFds<'_>> {
        self.session.as_ref().map(Session::fds)
    }

    pub fn next_deadline(&self) -> Option<Micros> {
        self.session.as_ref().and_then(Session::next_deadline)
    }

    pub fn on_discovery(
        &mut self,
        now: Micros,
        event: DiscoveryEvent,
        observer: &mut dyn SessionObserver,
    ) {
        match event {
            DiscoveryEvent::MouseAppeared(nodes) => self.mouse_appeared(now, nodes, observer),
            DiscoveryEvent::MouseGone { key } => self.mouse_gone(now, &key, observer),
        }
    }

    fn mouse_appeared(
        &mut self,
        now: Micros,
        nodes: MouseNodes,
        observer: &mut dyn SessionObserver,
    ) {
        match self.known.iter_mut().find(|known| known.key == nodes.key) {
            Some(known) if *known == nodes => {}
            Some(known) => *known = nodes.clone(),
            None => self.known.push(nodes.clone()),
        }
        let replace = match self.session.as_mut() {
            None => true,
            Some(session) if session.nodes().key == nodes.key => {
                if session.nodes() == &nodes
                    && matches!(
                        session.state(),
                        SessionState::Acquiring | SessionState::Active
                    )
                {
                    false
                } else {
                    session.release(ReleaseReason::Replaced, observer);
                    true
                }
            }
            Some(session) => session.state() == SessionState::Absent,
        };
        if replace {
            self.start(now, nodes, observer);
        }
    }

    fn mouse_gone(&mut self, now: Micros, key: &Path, observer: &mut dyn SessionObserver) {
        self.known.retain(|known| known.key != key);
        let Some(session) = self.session.as_mut() else {
            return;
        };
        if session.nodes().key != key {
            return;
        }
        session.release(ReleaseReason::Removed, observer);
        self.session = None;
        if let Some(next) = self.known.first().cloned() {
            self.start(now, next, observer);
        }
    }

    fn start(&mut self, now: Micros, nodes: MouseNodes, observer: &mut dyn SessionObserver) {
        let mut session = Session::new(nodes, self.mode, self.config.clone(), now);
        session.on_timer(now, &mut self.opener, observer);
        self.session = Some(session);
    }

    pub fn on_timer(&mut self, now: Micros, observer: &mut dyn SessionObserver) {
        if let Some(session) = self.session.as_mut() {
            session.on_timer(now, &mut self.opener, observer);
        }
    }

    pub fn on_raw_readable(&mut self, now: Micros, observer: &mut dyn SessionObserver) {
        if let Some(session) = self.session.as_mut() {
            session.on_raw_readable(now, observer);
        }
    }

    pub fn on_stock_readable(&mut self, now: Micros, observer: &mut dyn SessionObserver) {
        if let Some(session) = self.session.as_mut() {
            session.on_stock_readable(now, observer);
        }
    }

    pub fn shutdown(&mut self, observer: &mut dyn SessionObserver) {
        if let Some(mut session) = self.session.take() {
            session.release(ReleaseReason::Shutdown, observer);
        }
    }
}

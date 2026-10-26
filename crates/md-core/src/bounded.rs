use std::fmt;
use std::ops::Deref;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct BoundedVec<T: Copy + Default, const N: usize> {
    items: [T; N],
    len: usize,
}

impl<T: Copy + Default, const N: usize> BoundedVec<T, N> {
    pub fn new() -> Self {
        BoundedVec {
            items: [T::default(); N],
            len: 0,
        }
    }

    pub const fn capacity(&self) -> usize {
        N
    }

    pub fn push(&mut self, item: T) -> Result<(), T> {
        if self.len == N {
            return Err(item);
        }
        self.items[self.len] = item;
        self.len += 1;
        Ok(())
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }

    pub fn as_slice(&self) -> &[T] {
        &self.items[..self.len]
    }
}

impl<T: Copy + Default, const N: usize> Default for BoundedVec<T, N> {
    fn default() -> Self {
        BoundedVec::new()
    }
}

impl<T: Copy + Default, const N: usize> Deref for BoundedVec<T, N> {
    type Target = [T];

    fn deref(&self) -> &[T] {
        self.as_slice()
    }
}

impl<T: Copy + Default + fmt::Debug, const N: usize> fmt::Debug for BoundedVec<T, N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.as_slice()).finish()
    }
}

impl<'a, T: Copy + Default, const N: usize> IntoIterator for &'a BoundedVec<T, N> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.as_slice().iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_fills_up_to_capacity() {
        let mut list: BoundedVec<u8, 2> = BoundedVec::new();
        assert!(list.is_empty());
        assert_eq!(list.push(1), Ok(()));
        assert_eq!(list.push(2), Ok(()));
        assert_eq!(list.push(3), Err(3));
        assert_eq!(list.as_slice(), &[1, 2]);
        assert_eq!(list.capacity(), 2);
    }

    #[test]
    fn clear_empties_and_equality_ignores_stale_items() {
        let mut list: BoundedVec<u8, 3> = BoundedVec::new();
        list.push(7).unwrap();
        list.clear();
        assert!(list.is_empty());
        assert_eq!(format!("{list:?}"), "[]");
        let mut other: BoundedVec<u8, 3> = BoundedVec::new();
        other.push(4).unwrap();
        list.push(4).unwrap();
        assert_eq!(list.as_slice(), other.as_slice());
    }
}

//! Fixed-capacity history for sparklines. In memory only, owned by the tray,
//! written only while the dropdown is open.

use std::collections::VecDeque;

pub struct Ring<T> {
    buf: VecDeque<T>,
    cap: usize,
}

impl<T> Ring<T> {
    pub fn new(cap: usize) -> Self {
        Self {
            buf: VecDeque::with_capacity(cap),
            cap: cap.max(1),
        }
    }

    pub fn push(&mut self, v: T) {
        if self.buf.len() == self.cap {
            self.buf.pop_front();
        }
        self.buf.push_back(v);
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.buf.iter()
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn clear(&mut self) {
        self.buf.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_keeps_only_the_most_recent_values() {
        let mut r: Ring<f32> = Ring::new(3);
        for v in [1.0, 2.0, 3.0, 4.0] {
            r.push(v);
        }
        assert_eq!(r.len(), 3);
        assert_eq!(r.iter().copied().collect::<Vec<f32>>(), vec![2.0, 3.0, 4.0]);
    }

    #[test]
    fn a_fresh_ring_is_empty_and_iterates_nothing() {
        let r: Ring<f32> = Ring::new(4);
        assert_eq!(r.len(), 0);
        assert_eq!(r.iter().count(), 0);
    }

    #[test]
    fn clear_drops_everything_without_reallocating() {
        let mut r: Ring<f32> = Ring::new(2);
        r.push(1.0);
        r.clear();
        assert_eq!(r.len(), 0);
        r.push(9.0);
        assert_eq!(r.iter().copied().collect::<Vec<f32>>(), vec![9.0]);
    }
}

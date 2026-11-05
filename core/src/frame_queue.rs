//! Cola acotada de unidades de acceso H.264/H.265 para baja latencia.
//!
//! - Si la cola se llena, descarta los frames más antiguos.
//! - Un keyframe nuevo descarta los P-frames anteriores: ya no sirven de referencia.
//! - Ningún frame se entrega antes del primer keyframe.

use std::collections::VecDeque;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub seq: u32,
    pub timestamp_us: u64,
    pub keyframe: bool,
    pub data: Vec<u8>,
}

#[derive(Debug)]
pub struct FrameQueue {
    frames: VecDeque<Frame>,
    capacity: usize,
    seen_keyframe: bool,
    dropped: u64,
}

impl FrameQueue {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "la capacidad debe ser > 0");
        Self {
            frames: VecDeque::with_capacity(capacity),
            capacity,
            seen_keyframe: false,
            dropped: 0,
        }
    }

    /// Inserta un frame. Devuelve `true` si ha solicitado un keyframe al emisor.
    pub fn push(&mut self, frame: Frame) -> bool {
        if frame.keyframe {
            self.seen_keyframe = true;
            // Un keyframe invalida todo lo anterior: no hace falta conservar P-frames.
            self.dropped += self.frames.len() as u64;
            self.frames.clear();
            self.frames.push_back(frame);
            return false;
        }
        if !self.seen_keyframe {
            // Sin keyframe no hay referencia válida: se descarta y se pide uno.
            self.dropped += 1;
            return true;
        }
        if self.frames.len() == self.capacity {
            self.frames.pop_front();
            self.dropped += 1;
        }
        self.frames.push_back(frame);
        false
    }

    pub fn pop(&mut self) -> Option<Frame> {
        self.frames.pop_front()
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    pub fn dropped(&self) -> u64 {
        self.dropped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(seq: u32, keyframe: bool) -> Frame {
        Frame {
            seq,
            timestamp_us: seq as u64 * 33_000,
            keyframe,
            data: vec![seq as u8],
        }
    }

    #[test]
    fn drops_p_frames_before_first_keyframe_and_requests_one() {
        let mut q = FrameQueue::new(4);
        assert!(q.push(f(1, false)));
        assert!(q.is_empty());
        assert_eq!(q.dropped(), 1);
    }

    #[test]
    fn keeps_order_and_drops_oldest_when_full() {
        let mut q = FrameQueue::new(3);
        q.push(f(1, true));
        for s in 2..=5 {
            assert!(!q.push(f(s, false)));
        }
        assert_eq!(q.len(), 3);
        assert_eq!(q.pop().unwrap().seq, 3);
        assert_eq!(q.pop().unwrap().seq, 4);
        assert_eq!(q.pop().unwrap().seq, 5);
        assert_eq!(q.dropped(), 2);
    }

    #[test]
    fn capacity_one_keeps_only_latest() {
        let mut q = FrameQueue::new(1);
        q.push(f(1, true));
        q.push(f(2, false));
        q.push(f(3, false));
        assert_eq!(q.len(), 1);
        assert_eq!(q.pop().unwrap().seq, 3);
        assert_eq!(q.dropped(), 2);
        assert!(q.pop().is_none());
    }

    #[test]
    fn keyframe_on_full_queue_replaces_everything() {
        let mut q = FrameQueue::new(3);
        q.push(f(1, true));
        q.push(f(2, false));
        q.push(f(3, false));
        assert_eq!(q.len(), 3);
        assert!(!q.push(f(4, true)));
        assert_eq!(q.len(), 1);
        assert_eq!(q.dropped(), 3);
        assert!(q.pop().unwrap().keyframe);
    }

    #[test]
    fn dropped_counts_pre_keyframe_overflow_and_flush() {
        let mut q = FrameQueue::new(2);
        assert!(q.push(f(1, false))); // 1: sin keyframe
        q.push(f(2, true));
        q.push(f(3, false));
        q.push(f(4, false)); // 2: desborda
        q.push(f(5, true)); // 3: vacía 2 frames => +2
        assert_eq!(q.dropped(), 1 + 1 + 2);
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn keyframe_flushes_pending_p_frames() {
        let mut q = FrameQueue::new(8);
        q.push(f(1, true));
        q.push(f(2, false));
        q.push(f(3, false));
        q.push(f(4, true));
        assert_eq!(q.len(), 1);
        assert_eq!(q.pop().unwrap().seq, 4);
        assert_eq!(q.dropped(), 3);
    }
}

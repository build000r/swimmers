use std::collections::VecDeque;

/// A fixed-capacity ring buffer that stores terminal output frames with
/// monotonically increasing sequence numbers. Used for replay on reconnect.
#[derive(Debug)]
pub struct ReplayRing {
    capacity: usize,
    frames: VecDeque<Frame>,
    next_seq: u64,
    total_bytes: usize,
}

#[derive(Debug, Clone)]
struct Frame {
    seq: u64,
    data: Vec<u8>,
}

impl ReplayRing {
    /// Create a new ring buffer with the given byte capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            frames: VecDeque::new(),
            next_seq: 1,
            total_bytes: 0,
        }
    }

    /// Push terminal output data into the ring. Returns the sequence number
    /// assigned to this frame. Evicts oldest frames as needed to stay within
    /// the byte capacity.
    pub fn push(&mut self, data: &[u8]) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;

        // Evict oldest frames until we have room (or the buffer is empty).
        // If a single frame exceeds capacity, we still store it (evicting everything else).
        while self.total_bytes + data.len() > self.capacity && !self.frames.is_empty() {
            if let Some(evicted) = self.frames.pop_front() {
                self.total_bytes -= evicted.data.len();
            }
        }

        self.total_bytes += data.len();
        self.frames.push_back(Frame {
            seq,
            data: data.to_vec(),
        });

        seq
    }

    /// Replay all frames starting from the given sequence number (inclusive).
    ///
    /// Returns `Some(vec)` with `(seq, data)` pairs if the requested seq is
    /// still in the buffer. Returns `None` if the requested seq has been
    /// evicted (truncated), meaning the client must do a full refresh.
    pub fn replay_from(&self, seq: u64) -> Option<Vec<(u64, Vec<u8>)>> {
        // If the buffer is empty and they ask for seq 1 (or our next_seq), that's fine - empty replay.
        if self.frames.is_empty() {
            return if seq >= self.next_seq {
                Some(Vec::new())
            } else {
                None
            };
        }

        let window_start = self.frames.front().map(|f| f.seq).unwrap_or(self.next_seq);

        if seq < window_start {
            // Requested data has been evicted.
            return None;
        }

        let result: Vec<(u64, Vec<u8>)> = self
            .frames
            .iter()
            .filter(|f| f.seq >= seq)
            .map(|f| (f.seq, f.data.clone()))
            .collect();

        Some(result)
    }

    /// The sequence number that will be assigned to the next push.
    pub fn latest_seq(&self) -> u64 {
        // The last assigned seq is next_seq - 1, but if nothing has been pushed yet,
        // return 0 to indicate "no data yet".
        if self.next_seq <= 1 {
            0
        } else {
            self.next_seq - 1
        }
    }

    /// The lowest sequence number still retained in the buffer.
    /// Returns `latest_seq() + 1` if the buffer is empty.
    pub fn window_start_seq(&self) -> u64 {
        self.frames.front().map(|f| f.seq).unwrap_or(self.next_seq)
    }

    /// Clear all retained frames while keeping the sequence counter monotonic.
    /// The next push will still get the next expected seq number.
    pub fn clear(&mut self) {
        self.frames.clear();
        self.total_bytes = 0;
    }

    /// Total bytes currently retained in the buffer.
    // TODO: re-evaluate when buffer memory metrics are exposed in the API
    #[allow(dead_code)]
    pub fn total_bytes_retained(&self) -> usize {
        self.total_bytes
    }

    /// Concatenate all retained frames into a UTF-8 string (lossy) representing
    /// the visible terminal text. Used for snapshot / screen capture.
    pub fn snapshot(&self) -> String {
        let total: usize = self.frames.iter().map(|f| f.data.len()).sum();
        let mut buf = Vec::with_capacity(total);
        for frame in &self.frames {
            buf.extend_from_slice(&frame.data);
        }
        String::from_utf8_lossy(&buf).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_and_replay() {
        let mut ring = ReplayRing::new(1024);
        let s1 = ring.push(b"hello ");
        let s2 = ring.push(b"world");

        assert_eq!(s1, 1);
        assert_eq!(s2, 2);
        assert_eq!(ring.latest_seq(), 2);

        let frames = ring.replay_from(1).unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].0, 1);
        assert_eq!(frames[0].1, b"hello ");
        assert_eq!(frames[1].0, 2);
        assert_eq!(frames[1].1, b"world");
    }

    #[test]
    fn replay_partial() {
        let mut ring = ReplayRing::new(1024);
        ring.push(b"first");
        ring.push(b"second");
        ring.push(b"third");

        let frames = ring.replay_from(2).unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].0, 2);
        assert_eq!(frames[1].0, 3);
    }

    #[test]
    fn eviction_under_pressure() {
        // Capacity is 10 bytes. Push frames that force eviction.
        let mut ring = ReplayRing::new(10);
        ring.push(b"aaaa"); // 4 bytes, seq 1
        ring.push(b"bbbb"); // 4 bytes, seq 2  (total 8)
        ring.push(b"cccc"); // 4 bytes, seq 3  (would be 12, evict seq 1 -> 8)

        assert_eq!(ring.window_start_seq(), 2);
        assert!(ring.replay_from(1).is_none()); // seq 1 evicted
        assert_eq!(ring.replay_from(2).unwrap().len(), 2);
    }

    #[test]
    fn snapshot_concatenates() {
        let mut ring = ReplayRing::new(1024);
        ring.push(b"hello ");
        ring.push(b"world");
        assert_eq!(ring.snapshot(), "hello world");
    }

    #[test]
    fn clear_resets_frames_keeps_seq() {
        let mut ring = ReplayRing::new(1024);
        ring.push(b"hello ");
        ring.push(b"world");
        assert_eq!(ring.latest_seq(), 2);

        ring.clear();
        assert_eq!(ring.total_bytes_retained(), 0);
        assert_eq!(ring.snapshot(), "");
        // Sequence counter continues monotonically.
        let s3 = ring.push(b"after clear");
        assert_eq!(s3, 3);
        assert_eq!(ring.latest_seq(), 3);
        // Replay from seq 3 works; earlier seqs are gone.
        assert!(ring.replay_from(1).is_none());
        assert_eq!(ring.replay_from(3).unwrap().len(), 1);
    }

    #[test]
    fn empty_ring() {
        let ring = ReplayRing::new(1024);
        assert_eq!(ring.latest_seq(), 0);
        assert_eq!(ring.window_start_seq(), 1);
        assert_eq!(ring.snapshot(), "");
        // Asking for seq 1 on empty ring: nothing has been pushed, seq 1 >= next_seq(1)
        assert_eq!(ring.replay_from(1).unwrap().len(), 0);
    }
}

#[derive(Debug, Clone)]
pub(super) struct Histogram {
    pub(super) buckets: Vec<u64>,
    pub(super) sum: f64,
    pub(super) count: u64,
}

impl Histogram {
    pub(super) fn new(bucket_count: usize) -> Self {
        Self {
            buckets: vec![0; bucket_count + 1],
            sum: 0.0,
            count: 0,
        }
    }

    pub(super) fn observe(&mut self, buckets: &[f64], value: f64) {
        let bucket_index = buckets
            .iter()
            .position(|bucket| value <= *bucket)
            .unwrap_or(buckets.len());
        self.buckets[bucket_index] += 1;
        self.sum += value;
        self.count += 1;
    }
}

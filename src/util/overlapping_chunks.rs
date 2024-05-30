pub struct OverlappingChunks<'a, T> {
    slice: &'a [T],
    chunk_size: usize,
    step_size: usize,
    current_start: usize,
}

impl<'a, T> OverlappingChunks<'a, T> {
    pub(crate) fn new(slice: &'a [T], chunk_size: usize) -> Self {
        Self {
            slice,
            chunk_size,
            step_size: chunk_size - 1,
            current_start: 0,
        }
    }
}

impl<'a, T> Iterator for OverlappingChunks<'a, T> {
    type Item = &'a [T];

    fn next(&mut self) -> Option<Self::Item> {
        if self.current_start >= self.slice.len() {
            return None;
        }

        let end = (self.current_start + self.chunk_size).min(self.slice.len());
        let chunk = &self.slice[self.current_start..end];
        self.current_start += self.step_size;

        Some(chunk)
    }
}

pub fn last_n_items<I>(iter: I, n: usize) -> Vec<I::Item>
    where
        I: IntoIterator,
        I::Item: Clone,
{
    let items: Vec<I::Item> = iter.into_iter().collect();
    let len = items.len();
    if len <= n {
        items
    } else {
        items[len - n..].to_vec()
    }
}

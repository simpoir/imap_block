/// A never-ending slice iterator, for throttling retry operations.
pub struct Backoff<'a> {
    i: usize,
    v: &'a [u64],
}

impl<'a> Backoff<'a> {
    pub fn new(v: &'a [u64]) -> Backoff<'a> {
        Backoff { i: 0, v }
    }

    pub fn next(&mut self) -> u64 {
        let ret = self.v[self.i];
        self.i = std::cmp::min(self.i + 1, self.v.len() - 1);
        ret
    }

    pub fn reset(&mut self) {
        self.i = 0;
    }
}

#[cfg(test)]
mod tests {

    #[test]
    #[should_panic]
    fn test_empty() {
        super::Backoff::new(&[]).next();
    }

    #[test]
    fn test_single() {
        let mut b = super::Backoff::new(&[2]);
        assert_eq!(2, b.next());
        assert_eq!(2, b.next());
        assert_eq!(2, b.next());
    }

    #[test]
    fn test_advance() {
        let mut b = super::Backoff::new(&[1, 2, 3]);
        assert_eq!(1, b.next());
        assert_eq!(2, b.next());
        assert_eq!(3, b.next());
        assert_eq!(3, b.next());
        b.reset();
        assert_eq!(1, b.next());
        assert_eq!(2, b.next());
    }
}

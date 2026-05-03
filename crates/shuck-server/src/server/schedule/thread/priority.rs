#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ThreadPriority {
    Worker,
    LatencySensitive,
}

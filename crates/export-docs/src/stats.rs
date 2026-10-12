/// What one conversion held besides the bytes it returned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunStats {
    /// High water of the parser, inflate window, and zip chunk. Not the output document.
    pub scratch_peak: usize,
    pub input_bytes: u64,
    pub output_bytes: u64,
}

use std::sync::Arc;
use tiktoken_rs::CoreBPE;

#[derive(Clone)]
pub struct TokenCounter {
    bpe: Arc<CoreBPE>,
}

impl TokenCounter {
    pub fn cl100k() -> Self {
        Self {
            bpe: Arc::new(tiktoken_rs::cl100k_base().expect("failed to load cl100k_base encoding")),
        }
    }

    pub fn o200k() -> Self {
        Self {
            bpe: Arc::new(tiktoken_rs::o200k_base().expect("failed to load o200k_base encoding")),
        }
    }

    pub fn count_tokens(&self, text: &str) -> u64 {
        self.bpe.encode_ordinary(text).len() as u64
    }
}

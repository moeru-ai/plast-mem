// Re-export async_openai types for consumers
pub use async_openai::types::chat::{
  ChatCompletionRequestMessage, ChatCompletionRequestSystemMessage,
  ChatCompletionRequestUserMessage,
};

mod cosine_similarity;
pub use cosine_similarity::cosine_similarity;

mod embed;
pub use embed::embed;

mod embed_batch;
pub use embed_batch::embed_batch;

mod generate_object;
pub use generate_object::generate_object;

mod generate_text;
pub use generate_text::generate_text;

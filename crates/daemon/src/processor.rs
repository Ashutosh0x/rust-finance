use async_trait::async_trait;

#[async_trait]
pub trait Processor<E>: Send + Sync {
    type Output;
    async fn process(&mut self, event: E) -> Self::Output;
}

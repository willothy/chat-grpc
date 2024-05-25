use std::net::SocketAddr;
use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio_stream::wrappers::ReceiverStream;
use tonic::async_trait;
use tonic::{transport::Server, Request, Response, Status};

pub mod streaming {
    tonic::include_proto!("streaming");
}

use streaming::streaming_service_server::{StreamingService, StreamingServiceServer};
use streaming::Message;

#[derive(Default)]
pub struct MessagingService {
    connections: Arc<DashMap<std::net::SocketAddr, mpsc::Sender<Result<Message, Status>>>>,
}

impl Clone for MessagingService {
    fn clone(&self) -> Self {
        MessagingService {
            connections: self.connections.clone(),
        }
    }
}

impl MessagingService {
    pub fn broadcast_message(&self, sender: SocketAddr, message: Message) {
        let mut tasks = self
            .connections
            .iter()
            .filter(|entry| *entry.key() != sender)
            .map(|entry| {
                let key = entry.key().clone();
                let sender = entry.value().clone();
                let message = message.clone();
                async move {
                    if let Err(e) = sender.send(Ok(message)).await {
                        eprintln!("Error sending message to {}: {:?}", key, e);
                    }
                }
            })
            .collect::<JoinSet<_>>();

        tokio::spawn(async move {
            while let Some(result) = tasks.join_next().await {
                if let Err(e) = result {
                    eprintln!("Error sending message: {:?}", e);
                }
            }
        });
    }
}

#[async_trait]
impl StreamingService for MessagingService
where
    Self: Sync + Send,
{
    type StreamMessagesStream = ReceiverStream<Result<Message, Status>>;

    async fn stream_messages(
        &self,
        request: Request<tonic::Streaming<Message>>,
    ) -> Result<Response<Self::StreamMessagesStream>, Status> {
        let id = request.remote_addr().expect("client address");

        let mut inbound = request.into_inner();
        let (tx, rx) = mpsc::channel(4);

        self.connections.insert(id, tx);

        println!("Client connected: {:?}", id);

        tokio::spawn({
            let self = self.clone();
            async move {
                loop {
                    match inbound.message().await {
                        Ok(received) => {
                            if let Some(message) = received {
                                if let Some(streaming::message::Payload::Content(_)) =
                                    message.payload
                                {
                                    self.broadcast_message(id, message);
                                }
                            } else {
                                println!("Client disconnected: {:?}", id);
                                self.connections.remove(&id);
                                break;
                            }
                        }
                        Err(e) => {
                            eprintln!("Error receiving message: {}", e.message());
                        }
                    }
                }
            }
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "[::1]:50051".parse()?;
    let streaming_service = MessagingService::default();

    Server::builder()
        .add_service(StreamingServiceServer::new(streaming_service))
        .serve(addr)
        .await?;

    Ok(())
}

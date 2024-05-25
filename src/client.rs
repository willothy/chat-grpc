use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen};
use crossterm::QueueableCommand;
use std::future::IntoFuture;
use std::io;
use std::sync::Arc;
use streaming::message::Payload;
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;
use tonic::Request;
use tui::backend::Backend as _;
use tui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    text::Span,
    widgets::{Block, Borders, Paragraph},
    Terminal,
};

pub mod streaming {
    tonic::include_proto!("streaming");
}

use streaming::streaming_service_client::StreamingServiceClient;
use streaming::Message;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    enable_raw_mode()?;
    let stdout = io::stdout();
    let mut backend = CrosstermBackend::new(stdout);
    backend.queue(EnterAlternateScreen)?;

    let terminal = Arc::new(Mutex::new(Terminal::new(backend)?));

    let (tx, rx) = mpsc::channel::<Message>(1);

    let (render_tx, mut render_rx) = mpsc::channel::<()>(1);

    // Trigger initial render
    render_tx.send(()).await?;

    let mut streaming = StreamingServiceClient::connect("http://[::1]:50051")
        .await?
        .stream_messages(Request::new(ReceiverStream::new(rx)))
        .await?
        .into_inner();

    let messages: Arc<RwLock<Vec<String>>> = Arc::new(RwLock::new(vec![]));
    let input = Arc::new(RwLock::new(String::new()));

    let stream_handle: JoinHandle<Result<(), String>> = tokio::task::spawn({
        let messages = Arc::clone(&messages);
        let render_tx = render_tx.clone();
        async move {
            loop {
                match streaming.next().await {
                    Some(Ok(msg)) => {
                        match msg.payload {
                            Some(Payload::Content(content)) => {
                                messages.write().await.push(format!("Server: {}", content));
                                render_tx.send(()).await.map_err(|e| {
                                    format!("Failed to send render signal: {:?}", e)
                                })?;
                            }
                            Some(Payload::Disconnect(())) => {
                                unreachable!("Server should never send disconnect messages.")
                            }
                            _ => {}
                        };
                    }
                    Some(Err(e)) => {
                        eprintln!("Error receiving message: {:?}", e);
                        break;
                    }
                    None => break,
                }
            }
            Ok(())
        }
    });

    let render_handle = tokio::task::spawn({
        let terminal = Arc::clone(&terminal);
        let messages = Arc::clone(&messages);
        let input = Arc::clone(&input);
        async move {
            while let Some(()) = render_rx.recv().await {
                let mut terminal = terminal.lock().await;

                let size = terminal.size().expect("terminal size");
                let [msg_area, input_area] = *Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Min(1), Constraint::Length(3)].as_ref())
                    .split(size)
                    .to_owned()
                else {
                    unreachable!("There should only ever be exactly two areas.")
                };

                let messages_widget = {
                    let lines = (msg_area.height as usize).saturating_sub(2);
                    let view = messages
                        .read()
                        .await
                        .iter()
                        .rev()
                        .take(lines)
                        .cloned()
                        .rev()
                        .fold(String::new(), |acc, m| acc + &m + "\n");
                    Paragraph::new(view)
                        .block(Block::default().borders(Borders::ALL).title("Messages"))
                };

                let input_widget = {
                    let input = input.read().await;
                    let content = if input.len() == 0 {
                        Span::styled("Type your message...", Style::default().fg(Color::Gray))
                    } else {
                        Span::raw(input.to_owned())
                    };
                    Paragraph::new(content)
                        .block(Block::default().borders(Borders::ALL).title("Input"))
                };

                terminal
                    .draw(|f| {
                        f.render_widget(messages_widget, msg_area);
                        f.render_widget(input_widget, input_area);
                    })
                    .ok();
            }
        }
    });

    loop {
        if crossterm::event::poll(std::time::Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                match (key.code, key.modifiers) {
                    (KeyCode::Enter, _) => {
                        messages
                            .write()
                            .await
                            .push(format!("You: {}", Arc::clone(&input).read().await));

                        tx.send(Message {
                            payload: Some(streaming::message::Payload::Content(
                                input.write().await.drain(..).collect(),
                            )),
                        })
                        .await?;
                    }
                    (KeyCode::Char('q') | KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                        break;
                    }
                    (KeyCode::Char(c), KeyModifiers::NONE) => input.write().await.push(c),
                    _ => {}
                };
                render_tx.send(()).await?;
            }
        }
    }

    tokio::spawn(async move {
        tx.send(Message {
            payload: Some(Payload::Disconnect(())),
        })
        .await?;
        stream_handle.abort();
        stream_handle.await.ok();
        Result::<(), mpsc::error::SendError<_>>::Ok(())
    })
    .into_future()
    .await??;

    render_handle.abort();

    terminal
        .lock()
        .await
        .backend_mut()
        .queue(crossterm::terminal::LeaveAlternateScreen)?
        .flush()?;

    disable_raw_mode()?;

    Ok(())
}

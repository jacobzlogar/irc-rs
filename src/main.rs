use irc_rs::rsi;
use lazy_static::lazy_static;
use regex::Regex;
use std::{collections::HashMap, fmt::Debug};

use bytes::BytesMut;
use tokio::{
    io::{self, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, ReadHalf, WriteHalf},
    net::TcpStream,
    sync::{broadcast, mpsc},
};

use tokio_native_tls::TlsConnector;
use tokio_native_tls::TlsStream;

#[derive(Debug)]
struct Plugin {
    trigger: String,
    reply: String,
    plugin_tx: broadcast::Sender<String>,
    plugin_rx: broadcast::Receiver<String>,
}

#[derive(Debug)]
struct Bot<T> {
    read_half: BufReader<ReadHalf<T>>,
    buf: BytesMut,
    write_half: WriteHalf<T>,
    channel: &'static str,
    read_tx: mpsc::Sender<BytesMut>,
    read_rx: mpsc::Receiver<BytesMut>,
    write_rx: broadcast::Receiver<String>,
    write_tx: broadcast::Sender<String>,
    plugins: HashMap<String, Plugin>,
}

struct Config<'a> {
    address: &'a str,
}

#[derive(Debug)]
enum IrcMessageType {
    PING,
    PRIVMSG,
}

#[derive(Debug)]
enum IrcResponseType {
    PONG,
    PRIVMSG,
}

#[derive(Debug)]
struct IrcResponse {
    variant: IrcResponseType,
    message: Option<String>,
}

impl TryFrom<&IrcMessage> for IrcResponse {
    type Error = &'static str;
    fn try_from(key: &IrcMessage) -> Result<IrcResponse, Self::Error> {
        match key.variant {
            IrcMessageType::PING => Ok(IrcResponse {
                variant: IrcResponseType::PONG,
                message: Some("PONG".to_string()),
            }),
            _ => Ok(IrcResponse {
                variant: IrcResponseType::PRIVMSG,
                message: Some(key.value.to_string()),
            }),
        }
    }
}

fn parse_colons(text: &str) -> String {
    lazy_static! {
        static ref RE: Regex = Regex::new("(?:.*?:){2}").unwrap();
    }
    RE.replace(&text.to_string(), "".to_string()).to_string()
}

#[derive(Debug)]
struct PluginResponse {
    value: String,
}
impl PluginResponse {}

#[derive(Debug)]
struct RsiPluginCommand {
    value: String,
}

impl TryFrom<IrcMessage> for RsiPluginCommand {
    type Error = &'static str;
    fn try_from<'a>(msg: IrcMessage) -> Result<RsiPluginCommand, Self::Error> {
        let text = msg.clean_text();
        if !text.starts_with(".rsi") {
            return Err("Not an rsi command");
        }
        Ok(RsiPluginCommand { value: text })
    }
}

#[derive(Debug)]
struct IrcMessage {
    variant: IrcMessageType,
    value: String,
}

impl IrcMessage {
    fn clean_text(self) -> String {
        parse_colons(&self.value.to_string()).to_string()
    }
}

impl TryFrom<BytesMut> for IrcMessage {
    type Error = &'static str;
    fn try_from<'a>(buf: BytesMut) -> Result<IrcMessage, Self::Error> {
        let key = String::from_utf8(buf.to_vec()).unwrap();
        if key.starts_with("PING") {
            let msg = Self {
                variant: IrcMessageType::PING,
                value: key.to_string(),
            };
            Ok(msg)
        } else {
            let msg = Self {
                variant: IrcMessageType::PRIVMSG,
                value: key.to_string(),
            };
            Ok(msg)
        }
    }
}

struct Irc {}
impl Irc {
    async fn connect<'a>(
        server: Config<'a>,
    ) -> Result<TlsStream<TcpStream>, Box<dyn std::error::Error>> {
        let connector: TlsConnector = tokio_native_tls::native_tls::TlsConnector::builder()
            .build()?
            .into();
        let stream =
            TcpStream::connect(format!("{}:{}", server.address.to_string(), "6697")).await?;
        let stream = connector.connect(&server.address, stream).await?;
        Ok(stream)
    }
}
impl<T: 'static + Send + Sync + AsyncReadExt + AsyncWriteExt> Bot<T> {
    async fn start(
        connection: T,
        channel: &'static str, // this should be changed
    ) -> Result<Bot<T>, Box<dyn std::error::Error>> {
        println!("creating bot");
        let (read_half, write_half) = io::split(connection);
        let (read_tx, mut read_rx) = mpsc::channel::<BytesMut>(32);
        let (write_tx, mut write_rx) = broadcast::channel(32);
        let buf = BytesMut::with_capacity(4096);
        let read_half = BufReader::new(read_half.into());
        let (plugin_tx, mut plugin_rx) = broadcast::channel(16);
        let rsi_plugin = Plugin {
            trigger: ".rsi".to_string(),
            reply: "rsi!".to_string(),
            plugin_tx,
            plugin_rx,
        };
        let mut plugins: HashMap<String, Plugin> = HashMap::new();
        plugins.insert(String::from("rsi_plugin"), rsi_plugin);
        let mut bot = Bot {
            read_half,
            write_half,
            channel,
            read_tx,
            read_rx,
            write_rx,
            write_tx,
            buf,
            plugins,
        };

        let _ = &bot.join().await;
        Ok(bot)
    }

    async fn join(&mut self) -> () {
        let join_msg = format!("/JOIN {}", self.channel);
        let _ = Bot::<T>::send_message(&mut self.write_half, &"USER bot".to_string()).await;
        let _ = Bot::<T>::send_message(&mut self.write_half, &"NICK bot".to_string()).await;
        let _ = Bot::<T>::send_message(&mut self.write_half, &"JOIN #channel".to_string()).await;
        // let _ = self.send_message(&"USER rsibot".to_string()).await;
        // let _ = self.send_message(&"NICK rsibot".to_string()).await;
        // let _ = self.send_message(&join_msg.to_string()).await;
    }

    async fn read_messages(
        read_half: &mut BufReader<ReadHalf<T>>,
        read_tx: &mpsc::Sender<BytesMut>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        while let Ok(message) = read_half.lines().next_line().await {
            read_tx
                .send(BytesMut::from(message.unwrap().as_str()))
                .await
                .expect("failed to send bytes");
        }
        Ok(())
    }

    async fn send_message(
        write_half: &mut WriteHalf<T>,
        msg: &String,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let msg = format!("{}\r\n", msg);
        println!("sending msg: {:?}", msg);
        let _ = write_half.write(msg.as_bytes()).await?;
        Ok(())
    }

    async fn spawn_handles(mut self) -> Result<(), Box<dyn std::error::Error>> {
        println!("spawning handles & joining channel");

        let manager = tokio::spawn(async {
            // let read_half = self.read_half;
            let read_handle = tokio::spawn(async move {
                loop {
                    //TODO: handle error
                    Bot::<T>::read_messages(&mut self.read_half, &self.read_tx).await;
                }
            });

            let channel = self.channel.to_owned();
            let write_handle = tokio::spawn(async move {
                loop {
                    while let Some(read_msg) = self.read_rx.recv().await {
                        let irc_msg = IrcMessage::try_from(read_msg).unwrap();
                        let resp = IrcResponse::try_from(&irc_msg);
                        let plugin_resp = RsiPluginCommand::try_from(irc_msg);
                        match resp.unwrap().message {
                            Some(v) => {
                                Bot::<T>::send_message(&mut self.write_half, &v.to_string()).await;
                            }
                            None => (),
                        }
                        match plugin_resp {
                            Ok(_) => {
                                let resp = rsi::connect("ethusdt@kline_15m".to_string()).await;
                                let plugin_msg = format!("PRIVMSG {} {:?}", channel, resp);
                                Bot::<T>::send_message(
                                    &mut self.write_half,
                                    &plugin_msg.to_string(),
                                )
                                .await;
                            }
                            Err(_) => (),
                        }
                    }
                    // drop(self.write_half);
                }
            });
            read_handle.await;
            write_handle.await;
        });
        manager.await;
        Ok(())
    }

    // pub fn write(self) -> Result<(), tokio::io>
}
#[tokio::main]
async fn main() {
    let config = Config { address: "" };

    let connection = Irc::connect(config).await.unwrap();
    let bot = Bot::start(connection, &"#testing").await.unwrap();

    bot.spawn_handles().await;
}

pub mod rsi {
    use serde::{Deserialize, Serialize};
    use std::env;

    use futures_util::{future, pin_mut, StreamExt};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};

    #[derive(Serialize, Deserialize)]
    struct BinancePayload<B> {
        method: String,
        params: Vec<B>,
        id: u32,
    }

    pub async fn write_msg<B: Clone + Serialize>(
        payloads: Vec<BinancePayload<B>>,
        stdin_tx: futures_channel::mpsc::UnboundedSender<Message>,
    ) {
        payloads.iter().for_each(|payload| {
            let json = serde_json::to_string(payload).unwrap();
            stdin_tx.unbounded_send(Message::text(json));
        })
    }

    pub async fn connect(symbols: Vec<String>) {
        let connect_addr = "wss://stream.binance.com:9443/ws/ethusdt@miniticker".to_string();

        let url = url::Url::parse(&format!("{}", connect_addr)).unwrap();

        let (stdin_tx, stdin_rx) = futures_channel::mpsc::unbounded();
        tokio::spawn(read_stdin(stdin_tx));

        let (ws_stream, _) = connect_async(url).await.expect("Failed to connect");
        println!("WebSocket handshake has been successfully completed");

        let (write, read) = ws_stream.split();

        let subscribe = BinancePayload {
            method: "SUBSCRIBE".to_string(),
            params: symbols.to_owned(),
            id: 1,
        };

        let set_property = BinancePayload {
            method: "SET_PROPERTY".to_string(),
            params: ["combined".to_string(), "true".to_string()]
                .to_vec()
                .to_owned(),
            id: 1,
        };

        write_msg([subscribe, set_property].to_vec(), stdin_tx).await;

        let stdin_to_ws = stdin_rx.map(Ok).forward(write);
        let ws_to_stdout = {
            read.for_each(|message| async {
                let data = message.unwrap().into_data();
                tokio::io::stdout().write_all(&data).await.unwrap();
            })
        };

        pin_mut!(stdin_to_ws, ws_to_stdout);
        future::select(stdin_to_ws, ws_to_stdout).await;
    }

    // Our helper method which will read data from stdin and send it along the
    // sender provided.
    async fn read_stdin(tx: futures_channel::mpsc::UnboundedSender<Message>) {
        let mut stdin = tokio::io::stdin();
        loop {
            let mut buf = vec![0; 1024];
            let n = match stdin.read(&mut buf).await {
                Err(_) | Ok(0) => break,
                Ok(n) => n,
            };
            buf.truncate(n);
            tx.unbounded_send(Message::binary(buf)).unwrap();
        }
    }
    pub async fn validate_ticker(ticker: &str) {}
    pub async fn spawn_tasks() {}
}

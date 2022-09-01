#[tokio::main]
async fn main() {
    let symbols = vec!["btcusdt@aggTrade".to_string(), "btcusdt@depth".to_string()];
    irc_rs::rsi::connect(symbols).await;
}

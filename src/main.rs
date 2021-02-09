mod client;
mod error;
mod types;

use tokio::runtime;
use getopts::Options;
use std::{
  sync::Arc,
  io::{self, BufRead}
};

use crate::client::Client;

fn print_usage(program: &str, opts: Options) {
  let brief = format!("Usage: {} [options]", program);
  print!("{}", opts.usage(&brief));
}
  
fn main() -> Result<(), Box<dyn std::error::Error>> {
  let args: Vec<String> = std::env::args().collect();
  let program = args[0].clone();

  let mut opts = Options::new();
  opts.optopt("c", "connect", "url to which to connect", "url");
  opts.optopt("i", "id", "mqtt client id", "id");
  opts.optopt("u", "username", "username for the client", "username");
  opts.optopt("p", "password", "password for the client", "password");
  opts.optopt("t", "topic", "topic to subscribe", "topic");
  opts.optflag("s", "subscribe", "use as subscriber");
  opts.optopt("q", "qos", "quality of service", "qos"); 
  opts.optflag("r", "retain", "whether to publish with retain");
  opts.optflag("h", "help", "print this help menu");

  let matches = match opts.parse(&args[1..]) {
    Ok(m) => { m }
    Err(f) => { panic!(f.to_string()) }
  };

  if matches.opt_present("h") {
    print_usage(&program, opts);
    return Ok(());
  }

  let addr = matches.opt_str("c").unwrap();
  let client_id = matches.opt_str("i"); 
  let username = matches.opt_str("u");
  let password = matches.opt_str("p");
  let topic = matches.opt_str("t").unwrap();
  let qos = matches.opt_str("q").unwrap_or("0".to_owned()).parse::<u8>().unwrap();

  let use_as_subscriber = matches.opt_present("s");
  let retain = matches.opt_present("r");

  let rt = runtime::Runtime::new()?;

  rt.block_on(async {
    let client = Arc::new(Client::new(addr, client_id, username, password).await.unwrap());
    let listener = client.clone();
    tokio::spawn(async move {
      listener.run().await.unwrap();
    });
    
    if use_as_subscriber {
      let subscriber = client.clone();
      tokio::spawn(async move {
        subscriber.subscribe(&topic, qos).await.unwrap();
        println!("Subscribed to {}", topic);
      });

      while let Some(packet) = client.next().await {
        println!("Received topic: {} \n{}", packet.topic, packet.payload);
      }
    } else {
      let publisher = client.clone();
      let stdin = io::stdin();
      for line in stdin.lock().lines() {
        let payload = line.unwrap();
        publisher.publish(&topic, &payload, qos, retain).await.unwrap();
      }
    }
  });
  Ok(())
}
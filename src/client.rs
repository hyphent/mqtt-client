use tokio_util::codec::Framed;
use tokio::{
  net::{ToSocketAddrs, TcpStream},
  sync::{
    Mutex, 
    mpsc::{channel, Sender, Receiver}
  },
  time::{self, Duration}
};

use std::{
  sync::Arc,
  collections::HashMap
};

use futures_util::{
  sink::SinkExt, 
  stream::{StreamExt, SplitStream, SplitSink}
};

use uuid::Uuid;
use rand::Rng;

use mqtt_codec::{
  types::*,
  codec::MQTTCodec
};

use crate::types::{Result, Error};

const PONG_TIMEOUT: u64 = 1000;
const PING_INTERVAL: u16 = 10000;

pub struct Client {
  pub sink: Arc<Mutex<SplitSink<Framed<TcpStream, MQTTCodec>, DecodedPacket>>>,
  pub stream: Arc<Mutex<SplitStream<Framed<TcpStream, MQTTCodec>>>>,
  packets: Arc<Mutex<HashMap<u16, Sender<DecodedPacket>>>>,
  publish_rx: Arc<Mutex<Receiver<PublishPacket>>>,
  publish_tx: Arc<Sender<PublishPacket>>,
  ping_rx: Arc<Mutex<Receiver<PingRespPacket>>>,
  ping_tx: Arc<Sender<PingRespPacket>>
}

impl Client {
  pub async fn new<T: ToSocketAddrs + std::string::ToString>(addr: T, client_id: Option<String>, username: Option<String>, password: Option<String>) -> Result<Client> {
    let socket = TcpStream::connect(addr).await?;
    let mut stream = Framed::new(socket, MQTTCodec {});

    let client_id = client_id.unwrap_or(format!("mqtt_{}", Uuid::new_v4().to_string()));

    stream.send(DecodedPacket::Connect(ConnectPacket {
      client_id,
      clean_start: true,

      will_config: None,

      keep_alive: PING_INTERVAL, 

      username,
      password,

      properties: vec![]
    })).await?;

    if let Some(Ok(DecodedPacket::Connack(packet))) = stream.next().await {
      if packet.reason_code == ReasonCode::NotAuthorized {
        return Err(Error::NotAuthorized);
      }
    } else {
      return Err(Error::ConnectionLost);
    }

    let (sink, stream) = stream.split();

    let (publish_tx, publish_rx) = channel(100);
    let (ping_tx, ping_rx) = channel(1);
    
    Ok(Client {
      sink: Arc::new(Mutex::new(sink)),
      stream: Arc::new(Mutex::new(stream)),
      packets: Arc::new(Mutex::new(HashMap::new())),
      publish_rx: Arc::new(Mutex::new(publish_rx)),
      publish_tx: Arc::new(publish_tx),
      ping_rx: Arc::new(Mutex::new(ping_rx)),
      ping_tx: Arc::new(ping_tx)
    })
  }

  pub async fn run(&self) -> Result<()> {
    tokio::select! {
      _ = self.ping() => (),
      _ = self.listen() => ()
    }
    Ok(())
  }

  async fn listen(&self) -> Result<()> {
    while let Some(Ok(packet)) = self.stream.lock().await.next().await {
      match packet {
        DecodedPacket::Suback(packet) => {
          self.packets.lock().await.get_mut(&packet.packet_id).unwrap().send(DecodedPacket::Suback(packet.clone()))
            .await.map_err(|_| Error::MPSCError)?;
        }
        DecodedPacket::Puback(packet) => {
          self.packets.lock().await.get_mut(&packet.packet_id).unwrap().send(DecodedPacket::Puback(packet.clone()))
            .await.map_err(|_| Error::MPSCError)?;
        }
        DecodedPacket::Publish(packet) => {
          self.publish_tx.send(packet).await.map_err(|_| Error::MPSCError)?;
        }
        DecodedPacket::PingResp(packet) => {
          self.ping_tx.send(packet).await.map_err(|_| Error::MPSCError)?;
        }
        _ => ()
      }
    }
    Ok(())
  }

  pub async fn next(&self) -> Option<PublishPacket> {
    self.publish_rx.lock().await.recv().await
  }
  
  async fn handle_pingresp(&self) -> Result<()> {
    match self.ping_rx.lock().await.recv().await {
      Some(_) => Ok(()),
      _ => Err(Error::SomethingWentWrong)
    }
  }

  pub async fn ping(&self) -> Result<()> {
    let mut interval = time::interval(time::Duration::from_millis(PING_INTERVAL as u64));

    while let Ok(_) = {
      interval.tick().await;
      self.sink.lock().await.send(DecodedPacket::PingReq(PingReqPacket {})).await?;
      time::timeout(Duration::from_millis(PONG_TIMEOUT), self.handle_pingresp())
        .await.map_err(|_| Error::TimeoutError)?
    } {}

    Ok(())
  }

  fn generate_packet_id(&self) -> u16 {
    let mut rng = rand::thread_rng();
    rng.gen()
  }

  pub async fn publish(&self, topic: &str, payload: &str, qos: u8, retain: bool) -> Result<()> {
    let packet_id = match qos {
      0 => None,
      _ => Some(self.generate_packet_id())
    };

    self.sink.lock().await.send(DecodedPacket::Publish(PublishPacket {
      topic: topic.to_owned(),
      packet_id,
      payload: payload.to_owned(),
      config: PublishConfig {
        dup: false,
        qos,
        retain
      },
      properties: vec![]
    })).await?;


    if qos > 0 {
      let packet_id = packet_id.unwrap();
      let (tx, mut rx) = channel(1);
      self.packets.lock().await.insert(packet_id, tx);

      match rx.recv().await {
        Some(DecodedPacket::Puback(packet)) => {
          self.packets.lock().await.remove(&packet_id);
          match packet.reason_code {
            ReasonCode::NotAuthorized => return Err(Error::NotAuthorized),
            _ => ()
          };
        }
        _ => return Err(Error::SomethingWentWrong)
      }
    }
    Ok(())
  }

  pub async fn subscribe(&self, topic: &str, qos: u8) -> Result<()> {
    let packet_id = self.generate_packet_id();
    self.sink.lock().await.send(DecodedPacket::Subscribe(SubscribePacket {
      packet_id,
      subscriptions: vec![SubscriptionConfig{
        topic: topic.to_owned(),
        retain_handling: 0,
        rap: false,
        nl: false,
        qos
      }],
      properties: vec![]
    })).await?;

    let (tx, mut rx) = channel(1);
    self.packets.lock().await.insert(packet_id, tx);

    match rx.recv().await {
      Some(DecodedPacket::Suback(packet)) => {
        self.packets.lock().await.remove(&packet_id);
        for reason_code in packet.reason_codes {
          match reason_code {
            ReasonCode::NotAuthorized => return Err(Error::NotAuthorized),
            _ => ()
          };
        }
      }
      _ => return Err(Error::SomethingWentWrong)
    }

    Ok(())
  }
}
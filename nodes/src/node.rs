extern crate core_p2p;
extern crate futures;
extern crate tokio;
#[macro_use]
extern crate crossbeam_channel;
extern crate env_logger;
#[macro_use]
extern crate log;
extern crate serde;
extern crate serde_json;

#[macro_use]
extern crate serde_derive;


use core_p2p::{
    custom_proto::encode_decode::Request,
    secio,
    service::{build_service, ServiceEvent, ServiceHandle},
    Multiaddr,
};
use crossbeam_channel::{unbounded, Receiver, Sender};
use futures::prelude::*;
use futures::sync::mpsc::{unbounded as future_unbounded, UnboundedReceiver, UnboundedSender};
use std::{env, str, thread};
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize)]
enum MessageType {
    ShareAddr,
    ReturnNodeAddrs
}

#[derive(Debug, Serialize, Deserialize)]
struct Message {
    mtype: MessageType,
    //data: Vec<u8>,
    data: String,
    timestamp: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct NodeAddrs {
    pub node_addrs: HashMap<String, usize>,
}


#[allow(dead_code)]
enum Task {
    Dial(Multiaddr),
    Listen(Multiaddr),
    Disconnect(usize),
    Messages(Vec<(Vec<usize>, usize, Request)>),
}

struct Process {
    task_receiver: UnboundedReceiver<Task>,
    event_sender: Sender<ServiceEvent>,
    new_dialer: Vec<Multiaddr>,
    new_listen: Vec<Multiaddr>,
    disconnect: Vec<usize>,
    messages_buffer: Vec<(Vec<usize>, usize, Request)>,
    
}

impl Process {
    pub fn new() -> (Self, UnboundedSender<Task>, Receiver<ServiceEvent>) {
        let (task_sender, task_receiver) = future_unbounded();
        let (event_sender, event_receiver) = unbounded();

        (
            Process {
                task_receiver,
                event_sender,
                new_dialer: Vec::new(),
                new_listen: Vec::new(),
                disconnect: Vec::new(),
                messages_buffer: Vec::new(),
            },
            task_sender,
            event_receiver,
        )
    }
}

impl Stream for Process {
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<Option<()>, ()> {
        match self.task_receiver.poll()? {
            Async::Ready(Some(task)) => {
                match task {
                    Task::Dial(address) => self.new_dialer.push(address),
                    Task::Listen(address) => self.new_listen.push(address),
                    Task::Messages(messages) => self.messages_buffer.extend(messages),
                    Task::Disconnect(id) => self.disconnect.push(id),
                }
                Ok(Async::Ready(Some(())))
            }
            Async::Ready(None) => Ok(Async::Ready(None)),
            Async::NotReady => Ok(Async::NotReady),
        }
    }
}

impl ServiceHandle for Process {
    fn out_event(&self, event: Option<ServiceEvent>) {
        if let Some(event) = event {
            self.event_sender.send(event).unwrap();
        }
    }

    fn new_dialer(&mut self) -> Option<Multiaddr> {
        self.new_dialer.pop()
    }

    fn new_listen(&mut self) -> Option<Multiaddr> {
        self.new_listen.pop()
    }

    fn disconnect(&mut self) -> Option<usize> {
        self.disconnect.pop()
    }

    fn send_message(&mut self) -> Vec<(Vec<usize>, usize, Request)> {
        self.messages_buffer.drain(..).collect()
    }
}

fn main() {
    let log_env = env::var("RUST_LOG")
        .and_then(|value| Ok(format!("{},node=info", value)))
        .unwrap_or_else(|_| "node=info".to_string());
    env::set_var("RUST_LOG", log_env);
    env_logger::init();

    let key_pair = secio::SecioKeyPair::secp256k1_generated().unwrap();
    let (service_handle, task_sender, event_receiver) = Process::new();
    let mut service = build_service(key_pair, service_handle);
    let addr = service.listen_on("/ip4/127.0.0.1/tcp/0".parse().unwrap()).unwrap();

    let mut nas = NodeAddrs {
        node_addrs: HashMap::new() 
    };
    // for test now
    nas.node_addrs.insert(addr.clone().to_string(), 1);

    let mut dial_node = false;
    if let Some(to_dial) = std::env::args().nth(1) {
        //let _ = service.dial("/ip4/127.0.0.1/tcp/1337".parse().unwrap());
        println!("to dial {:?}", to_dial);
        let _ = service.dial(to_dial.parse().unwrap());
        dial_node = true;
    }

    let localaddr = addr.clone();
    thread::spawn(move || {
        info!("listening on {:?}", addr);
        tokio::run(service.map_err(|_| ()).for_each(|_| Ok(())))
    
    });
    

    loop {
        select!(
    /*
            recv(event_receiver) -> event => {
                match event {
                    Ok(event) => {
                        info!("--> {:?}", event);
                        match event {
                            ServiceEvent::CustomMessage {index, protocol, data} => {
                                if let Some(value) = data {
                                    info!("0 {:?}, {:?}, {:?}", index, protocol, str::from_utf8(&value));
                                }

                                let _ = task_sender.unbounded_send(Task::Messages(vec![(Vec::new(), 0, b"hello too".to_vec())]));
                            }
                            ServiceEvent::NodeInfo {index, endpoint, listen_address} => {
                                info!("{:?} {:?} {:?}", index, listen_address, endpoint);
                            }
                            _ => {}
                        }
                    }
                    Err(err) => error!("{}", err)
                }
            }
    */
            recv(event_receiver) -> event => {
                match event {
                    Ok(event) => {
                        info!("==> {:?}", event);
                        match event {
                            ServiceEvent::CustomProtocolOpen {index, protocol, version } => {
                                if dial_node {
                                    // here, send my addr
                                    let my_addr_msg = Message {
                                        mtype: MessageType::ShareAddr,
                                        //data: b"hello boy!".to_vec(),
                                        data: localaddr.to_string(),
                                        timestamp: 0
                                    };
                                    let msg_str = serde_json::to_string(&my_addr_msg).unwrap();
                                    let _ = task_sender.unbounded_send(Task::Messages(vec![(Vec::new(), 0, msg_str.into_bytes() )]));
                                }
                            },
                            ServiceEvent::NodeInfo {index, endpoint, listen_address} => {
                                info!("1 {:?} {:?} {:?}", index, listen_address, endpoint);
                            },
                            ServiceEvent::CustomMessage {index, protocol, data } => {
                                if let Some(value) = data {
                                    info!("1 {:?}, {:?}, {:?}", index, protocol, str::from_utf8(&value));
                                    let value_str = str::from_utf8(&value).unwrap();
                                    let msg: Message = serde_json::from_str(value_str).unwrap();
                                    info!("{:?}", msg);
                                    // here, parse message
                                    // check message type
                                    match msg.mtype {
                                        MessageType::ShareAddr => {
                                            let addr = msg.data;
                                            info!("{:?}", addr);
                                            nas.node_addrs.entry(addr).or_insert(0);

                                            info!("{:?}", nas);

                                            // XXX: here, contains the from addr just now, but for test
                                            // now
                                            let addr_list_str = serde_json::to_string(&nas.node_addrs).unwrap(); 
                                            let return_addrs_msg = Message {
                                                mtype: MessageType::ReturnNodeAddrs,
                                                //data: b"hello boy!".to_vec(),
                                                data: addr_list_str,
                                                timestamp: 0
                                            };
                                            let return_addrs_msg = serde_json::to_string(&return_addrs_msg).unwrap(); 
                                            let _ = task_sender.unbounded_send(Task::Messages(vec![(Vec::new(), 0, return_addrs_msg.into_bytes() )]));

                                        },
                                        MessageType::ReturnNodeAddrs => {
                                            let addrs = msg.data;
                                            info!("{:?}", addrs);

                                        }
                                    }

                                }
                            },
                            _ => {}
                        }
                    },
                    Err(err) => error!("{}", err)
                }
            }
        )
    }
}

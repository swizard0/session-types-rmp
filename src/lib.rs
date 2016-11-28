extern crate session_types_ng;
extern crate serde;
extern crate rmp_serde;

use std::io;
use std::io::{Read, Write};
use serde::{Serialize, Deserialize};
use session_types_ng::{ChannelSend, ChannelRecv, Carrier, HasDual, Chan};

pub trait RWChannel : Read + Write { }
impl<T> RWChannel for T where T: Read + Write {}

pub struct Channel {
    rw: Box<RWChannel + 'static>,
}

impl Channel {
    pub fn new<C>(rw: C) -> Channel where C: RWChannel + 'static {
        Channel {
            rw: Box::new(rw),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Value<T>(T);

impl<T> Value<T> where T: Serialize + Deserialize {
    pub fn new(value: T) -> Value<T> {
        Value(value)
    }
}

#[derive(Debug)]
pub enum SendError {
    Encode(rmp_serde::encode::Error),
    Flush(io::Error),
}

impl<T> ChannelSend for Value<T> where T: Serialize {
    type Crr = Channel;
    type Err = SendError;

    fn send(self, carrier: &mut Self::Crr) -> Result<(), Self::Err> {
        self.0.serialize(&mut rmp_serde::Serializer::new(&mut carrier.rw))
            .map_err(SendError::Encode)?;
        carrier.rw.flush()
            .map_err(SendError::Flush)
    }
}

#[derive(Debug)]
pub enum RecvError {
    Decode(rmp_serde::decode::Error),
}

impl<T> ChannelRecv for Value<T> where T: Deserialize {
    type Crr = Channel;
    type Err = RecvError;

    fn recv(carrier: &mut Self::Crr) -> Result<Self, Self::Err> {
        let value = Deserialize::deserialize(&mut rmp_serde::Deserializer::new(&mut carrier.rw))
            .map_err(RecvError::Decode)?;
        Ok(Value(value))
    }
}


#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
    }
}

extern crate session_types_ng;
extern crate serde;
extern crate rmp_serde;

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

impl<T> Serialize for Value<T> where T: Serialize {
    fn serialize<S>(&self, serializer: &mut S) -> Result<(), S::Error> where S: serde::ser::Serializer {
        self.0.serialize(serializer)
    }
}

impl<T> Deserialize for Value<T> where T: Deserialize {
    fn deserialize<D>(deserializer: &mut D) -> Result<Self, D::Error> where D: serde::de::Deserializer {
        Ok(Value(Deserialize::deserialize(deserializer)?))
    }
}

impl<T> ChannelSend for Value<T> where T: Serialize {
    type Crr = Channel;
    type Err = rmp_serde::encode::Error;

    fn send(self, carrier: &mut Self::Crr) -> Result<(), Self::Err> {
        self.serialize(&mut rmp_serde::Serializer::new(&mut carrier.rw))
    }
}


#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
    }
}

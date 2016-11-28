extern crate session_types_ng;
extern crate serde;
extern crate rmp_serde;

use std::io;
use std::io::{Read, Write};
use serde::{Serialize, Deserialize};
use session_types_ng::{ChannelSend, ChannelRecv, Carrier};

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

impl<T> Value<T> {
    pub fn get(self) -> T {
        self.0
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

impl Carrier for Channel {
    type SendChoiceErr = SendError;
    fn send_choice(&mut self, choice: bool) -> Result<(), Self::SendChoiceErr> {
        Value(choice).send(self)
    }

    type RecvChoiceErr = RecvError;
    fn recv_choice(&mut self) -> Result<bool, Self::RecvChoiceErr> {
        Value::recv(self).map(|Value(value)| value)
    }
}

#[cfg(test)]
mod tests {
    use std::net;
    use std::thread::spawn;
    use session_types_ng::{Chan, Rec, Recv, Choose, Offer, More, Nil, End, Var, Z, HasDual};
    use super::{Channel, Value};

    type Proto =
        Offer<ProtoFind, More<Offer<End, Nil>>>;

    type ProtoFind =
        Recv<Value<isize>, Rec<ProtoScan>>;

    type ProtoScan =
        Offer<ProtoScanValue, More<Offer<End, Nil>>>;

    type ProtoScanValue =
        Recv<Value<isize>, ProtoScanResult>;

    type ProtoScanResult =
        Choose<Var<Z>, More<Choose<End, Nil>>>;

    type SrvProto = Proto;
    type CliProto = <SrvProto as HasDual>::Dual;

    fn server(chan: Chan<Channel, (), SrvProto>) -> (Channel, bool) {
        let chan = match chan.offer().option(Ok).option(Err).unwrap() {
            Ok(chan_proceed) => chan_proceed,
            Err(chan_stop) => return (chan_stop.shutdown(), true),
        };
        let (chan, vsample) = chan.recv().unwrap();
        let sample = vsample.get();

        let mut chan = chan.enter();
        loop {
            let maybe_next = chan
                .offer()
                .option(|chan_value| Ok(chan_value.recv().unwrap()))
                .option(|chan_stop| Err(chan_stop.shutdown()))
                .unwrap();
            match maybe_next {
                Ok((next_chan, vvalue)) =>
                    if vvalue.get() == sample {
                        // found it
                        return (next_chan.second().unwrap().shutdown(), false);
                    } else {
                        // not found
                        chan = next_chan.first().unwrap().zero();
                    },
                Err(carrier) =>
                    return (carrier, false),
            }
        }
    }

    fn client<I>(chan: Chan<Channel, (), CliProto>, sample: isize, values: I) -> (Channel, Option<usize>)
        where I: Iterator<Item = isize>
    {
        let mut chan = chan
            .first().unwrap()
            .send(Value::new(sample)).unwrap()
            .enter();
        for (i, value) in values.enumerate() {
            let maybe_next = chan
                .first().unwrap()
                .send(Value::new(value)).unwrap()
                .offer()
                .option(|chan_not_found| Err(chan_not_found.zero()))
                .option(|chan_found| Ok(chan_found.shutdown()))
                .unwrap();
            match maybe_next {
                Ok(carrier) =>
                    return (carrier, Some(i)),
                Err(next_chan) =>
                    chan = next_chan,
            }
        }
        (chan.second().unwrap().shutdown(), None)
    }

    #[test]
    fn tcp_comm() {
        let acceptor = net::TcpListener::bind("0.0.0.0:51791").unwrap();
        let _th = spawn(move || {
            let slave_stream = net::TcpStream::connect("0.0.0.0:51791").unwrap();
            let mut carrier = Channel::new(slave_stream);
            loop {
                let (next_carrier, shutdown) = server(Chan::new(carrier));
                if shutdown {
                    break;
                } else {
                    carrier = next_carrier;
                }
            }
        });

        let master_stream = acceptor.accept().unwrap().0;
        let carrier = Channel::new(master_stream);
        let (carrier, maybe_pos) =
            client(Chan::new(carrier), 3, [-1, 0, 1, 2, 3, 4].iter().cloned());
        assert_eq!(maybe_pos, Some(4));
        let (carrier, maybe_pos) =
            client(Chan::new(carrier), -2, [-1, 0, 1, 2, 3, 4].iter().cloned());
        assert_eq!(maybe_pos, None);
        let (carrier, maybe_pos) =
            client(Chan::new(carrier), 3, [].iter().cloned());
        assert_eq!(maybe_pos, None);
        let (carrier, maybe_pos) =
            client(Chan::new(carrier), 6, [-1, 0, 1, 2, 3, 4, 5, 6, 6, 7].iter().cloned());
        assert_eq!(maybe_pos, Some(7));

        let chan_to_close: Chan<_, (), CliProto> = Chan::new(carrier);
        chan_to_close.second().unwrap().close();
    }
}

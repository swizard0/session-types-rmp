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
    use session_types_ng::{Chan, Rec, Send, Recv, Choose, Offer, More, Nil, End, Var, Z, HasDual};
    use super::{Channel, Value};

    // Server initial prompt: either start value searching session or force quit.
    type Proto =
        Offer<ProtoFind, More<Offer<End, Nil>>>;

    // Receive a sample value to search for and then start searching loop.
    type ProtoFind =
        Recv<Value<isize>, Rec<ProtoScan>>;

    // Perform termination condition check on each loop iteration before receiving a value to compare.
    type ProtoScan =
        Offer<ProtoScanValue, More<Offer<End, Nil>>>;

    // Receive next value to check and then return comparison result.
    type ProtoScanValue =
        Recv<Value<isize>, ProtoScanResult>;

    // Comparison result is either fail (continue the loop in this case) or match (break the loop then
    // and return an index of matched value).
    type ProtoScanResult =
        Choose<Var<Z>, More<Choose<Send<Value<usize>, End>, Nil>>>;

    type SrvProto = Proto;
    type CliProto = <SrvProto as HasDual>::Dual;

    // This is the server session protocol handler. Upon exit, it returns two values tuple:
    //  * the underlying channel for futher reusing
    //  * a flag indicating whether client requests a force quit
    fn server(chan: Chan<Channel, (), SrvProto>) -> (Channel, bool) {
        // Process start protocol prompt
        let chan = match chan.offer().option(Ok).option(Err).unwrap() {
            // proceeding with value searching
            Ok(chan_proceed) => chan_proceed,
            // stopping the server
            Err(chan_stop) => return (chan_stop.shutdown(), true),
        };

        // Receive a sample value to search
        let (chan, vsample) = chan.recv().unwrap();
        let sample = vsample.get();

        // Enter the searching loop
        let mut chan = chan.enter();
        let mut values_count = 0;
        loop {
            let maybe_next = chan
                .offer()
                // client sends a value to compare
                .option(|chan_value| Ok(chan_value.recv().unwrap()))
                // client requests loop break
                .option(|chan_stop| Err(chan_stop.shutdown()))
                .unwrap();
            match maybe_next {
                Ok((next_chan, vvalue)) =>
                    // comparing the value requested with the sample
                    if vvalue.get() == sample {
                        // found it
                        let carrier = next_chan
                            // notify client about success
                            .second().unwrap()
                            // send the index of matched value
                            .send(Value::new(values_count)).unwrap()
                            // shutdown the channel and extract it's carrier
                            .shutdown();
                        return (carrier, false);
                    } else {
                        // not found, notify client about fail and continue the loop
                        chan = next_chan.first().unwrap().zero();
                    },
                Err(carrier) =>
                    return (carrier, false),
            }
            values_count += 1;
        }
    }

    // This is the client session protocol handler. Upon exit, it returns two values tuple:
    //  * the underlying channel for futher reusing
    //  * a possible value number that matched the sample provided
    fn client<I>(chan: Chan<Channel, (), CliProto>, sample: isize, values: I) -> (Channel, Option<usize>)
        where I: Iterator<Item = isize>
    {
        let mut chan = chan
            // choose the searching session
            .first().unwrap()
            // install sample value to search for
            .send(Value::new(sample)).unwrap()
            // enter the searching loop
            .enter();
        for value in values {
            let maybe_next = chan
                // notify server that there is a value to process
                .first().unwrap()
                // send the value
                .send(Value::new(value)).unwrap()
                .offer()
                // server responds about comparison fail
                .option(|chan_not_found| Err(chan_not_found.zero()))
                // server responds about comparison success
                .option(|chan_found| Ok(chan_found))
                .unwrap();
            match maybe_next {
                Ok(next_chan) => {
                    // in case of found value receive it's index and stop iteration
                    let (next_chan, vindex) = next_chan.recv().unwrap();
                    return (next_chan.shutdown(), Some(vindex.get()));
                },
                Err(next_chan) =>
                    // continue iteration in case of failure
                    chan = next_chan,
            }
        }
        // no more items: notify server about it and shutdown the channel
        (chan.second().unwrap().shutdown(), None)
    }

    #[test]
    fn tcp_comm() {
        let acceptor = net::TcpListener::bind("0.0.0.0:51791").unwrap();
        let _th = spawn(move || {
            // Background server thread: repeat the protocol session until
            // `server` function returns true.
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

        // Client requests: reuse the same underlying carrier
        // several times for various client protocol sessions.
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

        // Request server shutdown
        Chan::<_, (), CliProto>::new(carrier)
            .second().unwrap()
            .close();
    }
}

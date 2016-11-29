# session-types-rmp #

## Summary ##

A generic session types for RW IO channels using rmp-serde. See [session-types-ng](https://github.com/swizard0/session-types-ng) for details about session types.

Channels communication is performed using abstract carrier which implements both `Read` and `Write` traits, and transferred values are serialized using `rmp-serde` crate.

## Usage ##

`Cargo.toml`:

```toml
[dependencies]
session-types-rmp = "0.2"
```

To `src/main.rs`:

```
#!rust
extern crate session-types-rmp;
```

## Motivating example ##

Consider the following simple code snippet:

#!rust
fn search_index<I>(sample: isize, values: I) -> Option<usize>
    where I: Iterator<Item = isize>
{
    for (i, value) in values.enumerate() {
        if value == sample {
            return Some(i);
        }
    }
    None
}
```

Suppose we want to design client-server application, where the algorithm above is implemented in server part and all input values for it are provided by clients. All communication between server and client are handled by TCP connection. Let's start:

```
#!rust
    use std::net;
    use std::thread::spawn;
```

So when a client establishes a connection with server, an interactive session should be created in which both sides should talk with each other using some kind of protocol. Let's encode this session using [session types](https://github.com/swizard0/session-types-ng):

```
#!rust
    use session_types_ng::{Chan, Rec, Send, Recv, Choose, Offer, More, Nil, End, Var, Z, HasDual};
    use session_types_rmp::{Channel, Value};

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
```

Given such kind of protocol schema, we can easily code a server and client implementations:

```
#!rust
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

```
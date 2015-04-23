#![feature(unboxed_closures, std_misc)]

//! # The pipeline crate
//! This crate contains all functions for pipeline construction
//!
//! Pipelines are a very powerful concept that allow advanced dataflow design
//! through a program. They are also asynchronous and inherently threaded.
//!
//! ## Design
//! Pipelines are a thin abstraction layer on top of the channel infrastructure
//! already included in the standard library. Functions take Receivers as arguments and
//! return Receivers, meanwhile spawning threads that consume the passed receiver(s).
//! The pipeline functions can take transforming closures, predicates for selection,
//! or have no inherent logic at all (like (inverse) multiplexers). There is also
//! a number of functions taking no Receiver but instead feeding other things
//! (like iterators, line-by-line text, files or standard input) into a channel.
//! Lastly, there are endpoint functions that, for example, collect channel output
//! into lists or write them to standard output or a file.

#[macro_use]
extern crate log;

pub mod either;

use std::fmt;
use std::error;
use std::iter::FromIterator;
use std::ops::{Deref, DerefMut};
use std::result;
use std::thread;
use std::sync::Future;
use std::sync::mpsc::{Sender, Receiver, channel};
use either::Either;

/// Represents either a value or an error in the pipeline
pub type Result<T> = result::Result<T, Error>;

/// Represents a type of error for the pipeline error type
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    Other
}

/// Represents an error that occurred in the pipeline
#[derive(Copy, Clone, PartialEq, Eq)]
pub struct Error {
    /// An enum that represents the kind of error this is
    pub kind: ErrorKind,
    /// A human-readable description of the error
    pub desc: &'static str,
}

impl fmt::Display for Error {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        match self {
            &Error { ref kind, ref desc } => write!(fmt, "Pipeline error of kind {:?}: {}", kind, desc)
        }
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(self, fmt)
    }
}

impl error::Error for Error {
    fn description(&self) -> &str {
        self.desc
    }
}

/// Represents a Receiver of Result<T> elements
pub struct Pipeline<T: Send>(Receiver<Result<T>>);

impl <T: Send> Deref for Pipeline<T> {
    type Target = Receiver<Result<T>>;

    fn deref<'a>(&'a self) -> &'a Receiver<Result<T>> {
        &self.0
    }
}

impl <T: Send> DerefMut for Pipeline<T> {
    fn deref_mut<'a>(&'a mut self) -> &'a mut Receiver<Result<T>> {
        &mut self.0
    }
}

impl <T: Send + 'static> Pipeline<T> {

    /// Instantiates a new Pipeline with an input Sender
    pub fn new() -> (Sender<Result<T>>, Pipeline<T>) {
        let (i, o) = channel();
        (i, Pipeline(o))
    }

    /// Creates a channel and writes the given value to it
    ///
    /// This function creates a new channel and writes the
    /// given value into that channel. This is useful
    /// for starting a pipeline.
    pub fn from_value(value: T) -> Pipeline<T> {
        // Create a channel
        let (i, o) = Pipeline::new();

        // Write the given value into the channel
        i.send(Ok(value)).unwrap();

        // Return the receiver of the channel for further propagation
        o
    }

    /// Creates a channel and writes all values in the given Vec to it, consuming the Vec
    ///
    /// This function creates a new channel and writes all
    /// values in the given Vec to it, consuming the Vec.
    /// The writing happens asynchronously and on a separate thread
    /// This is useful for starting a pipeline.
    pub fn from_vec(values: Vec<T>) -> Pipeline<T> {
        // Create a channel
        let (i, o) = Pipeline::new();

        // Spawn a thread which writes all values in the given
        // Vec into the channel
        thread::spawn(move || {
            // Iterate over all values in the given Vec (consuming it in the process)
            for value in values.into_iter() {
                i.send(Ok(value)).unwrap();
            }
        });

        // Return the receiver of the channel for further propagation
        o
    }

    /// Creates a channel and writes all values in the given Vec to it synchronously, consuming the Vec
    ///
    /// This function creates a new channel and writes all
    /// values in the given Vec to it, consuming the Vec.
    /// The writing happens synchronously, before returning from the function.
    /// This is useful for starting a pipeline.
    pub fn from_vec_sync(values: Vec<T>) -> Pipeline<T> {
        // Create a channel
        let (i, o) = Pipeline::new();

        // Iterate over all values in the given Vec
        for value in values.into_iter() {
            i.send(Ok(value)).unwrap();
        }

        // Return the receiver of the channel for further propagation
        o
    }

    /// Creates a Pipeline out of an ordinary channel
    ///
    /// This function returns a new pipeline that gets fed all
    /// input on the Receiver that was given to the function.
    pub fn from_receiver(recv: Receiver<T>) -> Pipeline<T> {
        // Create a channel
        let (i, o) = Pipeline::new();

        // Spawn a thread that redirects all receiver output into the new channel
        thread::spawn(move || {
            // This loop goes on until the given receiver has closed
            for y in recv.iter() {
                i.send(Ok(y)).unwrap()
            }
            info!("Pipeline redirection finished because of closed channel")
        });

        // Return the receiver of the channel for further propagation
        o
    }

    /// Transforms receiver output using the passed operation and feeds it into a newly constructed pipeline
    ///
    /// This function takes an input Pipeline that produces objects of type I.
    /// Those objects are then transformed to type O using the closure op
    /// and fed into a newly created pipeline (this all happens on a new thread).
    /// Returned from the function is the Pipeline of the channel the
    /// transformed objects are fed into.
    pub fn map<O: Send + 'static, F: Send + 'static>(self: Pipeline<T>, mut op: F) -> Pipeline<O>
        where F: FnMut(T) -> Result<O> {
            // New channel for transformed output.
        let (i, o) = Pipeline::new();

        // Worker thread that transforms received objects
        thread::spawn(move || {
            // This loop goes on as long as the Sender corresponding to
            // the given receiver isn't closed.
            for x in self.iter() {
                // Check whether or not an error has occurred on the channel
                match x {
                    Ok(val) => {
                        // Check whether or not the operation produces an error
                        match op(val) {
                            Ok(result) => i.send(Ok(result)).unwrap(),
                            Err(err) => {
                                // Report and propagate
                                error!("{:?}", err);
                                i.send(Err(err)).unwrap();
                                break;
                            }
                        };
                    },
                    Err(err) => {
                        // Propagate any errors
                        i.send(Err(err)).unwrap();
                        break;
                    }
                }
            }

            info!("Stage finished because of closed channel")
        });

        // The output channel's Receiver is returned for further propagation
        o
    }

    /// Transforms receiver output using the passed operation and a mutable context and feeds it into a newly constructed pipeline
    ///
    /// This function takes an input Pipeline that produces objects of type I and a context.
    /// The objects from the pipeline are then transformed to type O using the closure op
    /// that also receives a mutable reference to the context
    /// and fed into a newly created pipeline (this all happens on a new thread).
    /// Returned from the function are a Future that will contain the context once the stage is done
    /// and the Pipeline of the channel the transformed objects are fed into.
    pub fn map_with_ctx<O: Send + 'static, F: Send + 'static, C: Send + 'static>(self: Pipeline<T>, mut op: F, ctx: C) -> (Future<C>, Pipeline<O>)
        where F: FnMut(T, &mut C) -> Result<O> {
        // New channel for transformed output
        let (i, o) = Pipeline::new();

        // Future that transforms received objects and contains context after finishing
        let f = Future::spawn(move || {
            let mut context = ctx;
            // This loop goes on as long as the Sender corresponding to
            // the given receiver isn't closed
            for x in self.iter() {
                // Check whether or not an error has occurred on the channel
                match x {
                    Ok(val) => {
                        match op(val, &mut context) {
                            Ok(result) => i.send(Ok(result)).unwrap(),
                            Err(err) => {
                                // Report and propagate
                                error!("{:?}", err);
                                i.send(Err(err)).unwrap();
                                break;
                            }
                        }
                    },
                    Err(err) => {
                        // Propagate any errors
                        i.send(Err(err)).unwrap();
                        break;
                    }
                }
            }

            info!("Stage finished because of closed channel");
            context
        });

        // The output channel's Receiver is returned for further propagation
        (f, o)
    }

    /// Executes a callback on each value received on the pipeline
    ///
    /// This function takes a pipeline and a callback
    /// and calls that callback with each value received
    /// on the channel.
    pub fn each<F: Send + 'static>(self: Pipeline<T>, mut callback: F)
        where F: FnMut(Result<T>) {
        // Worker thread that performs an operation on each received value
        thread::spawn(move || {
            // This loop goes on as long as the Sender corresponding to
            // the given receiver isn't closed
            for x in self.iter() {
                callback(x);
            }

            info!("Each object on pipeline has been processed and pipeline closed");
        });
    }

    /// Executes a callback on each value received on the pipeline, with a shared context.
    ///
    /// This function takes a pipeline, a callback and a context
    /// and calls that callback with each value received on the channel and a mutable ref to the context.
    /// Returned is a Future that will contain the context after the pipeline has been emptied.
    pub fn each_with_ctx<F: Send + 'static, C: Send + 'static>(self: Pipeline<T>, mut callback: F, ctx: C) -> Future<C>
        where F: FnMut(Result<T>, &mut C) {
        // Worker thread that performs an operation on each received value
        let f = Future::spawn(move || {
            let mut context = ctx;
            // This loop goes on as long as the Sender corresponding to
            // the given receiver isn't closed
            for x in self.iter() {
                // Acquire a lock on the context
                callback(x, &mut context);
            }

            info!("Each object on pipeline has been processed and pipeline closed");
            context
        });

        f
    }

    /// Multiplexes messages received on two separate pipelines into a newly created pipeline
    ///
    /// This function takes two input Pipelines and feeds messages on both of them
    /// into a pipeline it creates, without any further transformation.
    /// Returned from the function is the Pipeline object of the pipeline all messages
    /// are fed into.
    pub fn merge(self: Pipeline<T>, other: Pipeline<T>) -> Pipeline<T> {
        // New channel for combined output
        let (i, o) = Pipeline::new();

        // Worker thread that feeds output from the given Receivers into a single output channel
        thread::spawn(move || {
            // Check whether or not the left channel has been closed
            for x in self.iter() {
                // Check whether or not an error has occurred on the channel
                match x {
                    Ok(val) => { i.send(Ok(val)).unwrap(); },
                    Err(err) => {
                        // Propagate any errors
                        i.send(Err(err)).unwrap();
                        break;
                    }
                }
            }
            // Check whether or not the right channel has been closed
            for x in other.iter() {
                // Check whether or not an error has occurred on the channel
                match x {
                    Ok(val) => { i.send(Ok(val)).unwrap(); },
                    Err(err) => {
                        // Propagate any errors
                        i.send(Err(err)).unwrap();
                        break;
                    }
                }
            }
            info!("Mux finished because of closed channels");
        });

        // The output channel's Receiver is returned for further propagation
        o
    }

    /// Multiplexes messages received on two separate channels into a newly created channel, marked with an Either
    ///
    /// This function takes two input Receivers and feeds messages on both of them
    /// into a channel it creates, wrapping them in an Either.
    /// Returned from the function is the Receiver of the channel all messages
    /// are fed into.
    pub fn merge_either<R: Send + 'static>(self: Pipeline<T>, other: Pipeline<R>) -> Pipeline<Either<T, R>> {
        // New channel for combined output
        let (i, o) = Pipeline::new();

        // Worker thread that feeds output from both Receivers into a single output channel
        thread::spawn(move || {
            for x in self.iter() {
                // Check whether or not an error has occurred on the channel
                match x {
                    Ok(val) => { i.send(Ok(Either::Left(val))).unwrap(); },
                    Err(err) => {
                        // Propagate any errors
                        i.send(Err(err)).unwrap();
                        break;
                    }
                }
            }

            for x in other.iter() {
                // Check whether or not an error has occurred on the channel
                match x {
                    Ok(val) => { i.send(Ok(Either::Right(val))).unwrap(); },
                    Err(err) => {
                        // Propagate any errors
                        i.send(Err(err)).unwrap();
                        break;
                    }
                }
            }
            info!("Mux finished because of closed channels");
        });

        o
    }

    /// Feeds messages received on a single channel into either of two newly created channels, decided using given predicate
    ///
    /// This function takes a single input Receiver and feeds messages on that Receiver
    /// into one of two new channels it creates, decided using the output of the given predicate
    /// closure. If the closure returns true based on the message, it is forwarded into the
    /// left channel (first one in the tuple). If the closure returns false, it is forwarded into
    /// the right channel (second one in the tuple).
    pub fn split_by<F: Send + 'static>(self: Pipeline<T>, mut left: F) -> (Pipeline<T>, Pipeline<T>)
        where F: FnMut(&T) -> bool {
        // New channels for conditional output
        let (left_i, left_o) = Pipeline::new();
        let (right_i, right_o) = Pipeline::new();

        // Worker thread that feeds output from the given Receiver into either of the output channels
        thread::spawn(move || {
            // This loop goes on as long as the Sender corresponding to the
            // given receiver isn't closed
            for x in self.iter() {
                match x {
                    Ok(val) => {
                        if left(&val) {
                            left_i.send(Ok(val)).unwrap();
                        } else {
                            right_i.send(Ok(val)).unwrap();
                        }
                    },
                    Err(err) => {
                        // Propagate any errors
                        left_i.send(Err(err.clone())).unwrap();
                        right_i.send(Err(err)).unwrap();
                        break;
                    }
                }
            }
            info!("Selection finished because of closed channel");
        });

        // Both of the output channels' Receivers are returned for further propagation
        (left_o, right_o)
    }

    /// Feeds all non-errors from a pipeline into an ordinary receiver
    ///
    /// This function takes all input from a pipeline and filters
    /// all the errors from the pipeline, feeding all non-errors
    /// into a new channel.
    pub fn filter_ok(self: Pipeline<T>) -> Receiver<T> {
        // Create a channel
        let (i, o) = channel();

        // Spawn a thread that redirects all non-error receiver output into the new channel
        thread::spawn(move || {
            // This loop goes on until the given receiver has closed
            for x in self.iter() {
                match x {
                    Ok(val) => { i.send(val).unwrap(); },
                    Err(_) => ()
                }
            }
            info!("Filtering finished because of closed channel");
        });

        // Return the receiver of the channel for further propagation
        o
    }

    /// Feeds all errors from a pipeilne into an ordinary receiver
    ///
    /// This function takes all input from a pipeline and filters
    /// all the non-errors from the pipeline, feeding all errors
    /// into a new channel.
    pub fn filter_err(self: Pipeline<T>) -> Receiver<Error> {
        // Create a channel
        let (i, o) = channel();

        // Spawn a thread that redirects all error receiver output into the new channel
        thread::spawn(move || {
            // This loop goes on until the given receiver has closed
            for x in self.iter() {
                match x {
                    Ok(_) => (),
                    Err(err) => { i.send(err).unwrap(); }
                }
            }
            info!("Filtering finished because of closed channel");
        });

        // Return the receiver of the channel for further propagation
        o
    }

    /// Collects all non-errors from a pipeline into a Vec
    ///
    /// This function takes all input from a pipeline and filters
    /// all the errors from the pipeline, collecting all non-errors
    /// into a Vec.
    /// NOTE: This function blocks if the original input channel
    ///       of the pipeline hasn't been closed.
    ///       Make sure you drop that first (or use one of the
    ///       construction functions in this module).
    pub fn collect_ok(self: Pipeline<T>) -> Vec<T> {
        FromIterator::from_iter(
            self.iter().map(|r: Result<T>| -> Option<T> {
                    r.ok()
                }).filter(|o: &Option<T>| {
                    o.is_some()
                }).map(|o: Option<T>| -> T {
                    o.unwrap()
                })
        )
    }

    /// Collects all errors from a pipeline into a Vec
    ///
    /// This function takes all input from a pipeline and filters
    /// all the non-errors from the pipeline, collecting all errors
    /// into a Vec.
    /// NOTE: This function blocks if the original input channel
    ///       of the pipeline hasn't been closed.
    ///       Make sure you drop that first (or use one of the
    ///       construction functions in this module).
    pub fn collect_err(self: Pipeline<T>) -> Vec<Error> {
        FromIterator::from_iter(
            self.iter().map(|r: Result<T>| -> Option<Error> {
                    r.err()
                }).filter(|o: &Option<Error>| {
                    o.is_some()
                }).map(|o: Option<Error>| -> Error {
                    o.unwrap()
                })
        )
    }
}

impl <T: Send + Clone + 'static> Pipeline<T> {

    /// Inverse-multiplexes messages received on a single channel into two new channels
    ///
    /// This function takes a single input Receiver and feeds messages on that
    /// Receiver into two new channels it creates (cloning the message in the process).
    /// Returned from the function are two Receivers, both of which will receive
    /// the exact same messages.
    ///
    /// NOTE: this only works with input messages that implement the Clone trait
    pub fn duplicate(self: Pipeline<T>) -> (Pipeline<T>, Pipeline<T>) {
        // New channels for cloned output
        let (i1, o1) = Pipeline::new();
        let (i2, o2) = Pipeline::new();

        // Worker thread that feeds output from the given Receiver into both output channels
        thread::spawn(move || {
            // This loop goes on as long as the Sender corresponding to
            // the given receiver isn't closed.
            for x in self.iter() {
                // Check whether or not an error has occurred on the channel
                match x {
                    Ok(val) => {
                        i1.send(Ok(val.clone())).unwrap();
                        i2.send(Ok(val)).unwrap();
                    },
                    Err(err) => {
                        // Propagate any errors
                        i1.send(Err(err.clone())).unwrap();
                        i2.send(Err(err)).unwrap();
                        break;
                    }
                }
            }
            info!("Inverse mux finished because of closed channel");
        });

        // Both of the output channels' Receivers are returned for further propagation
        (o1, o2)
    }

}

impl <L: Send + 'static, R: Send + 'static> Pipeline<Either<L, R>> {

    /// Feeds messages received on a single channel into either of two newly created channels, decided based on its side of Either
    ///
    /// This function takes a single input Receiver and feeds messages on that Receiver
    /// into one of two new channels it creates, decided using the side of Either it is on.
    /// A Left goes into the left channel (first one in the tuple), a Right into the right
    /// channel (second one in the tuple).
    pub fn split_by_either(self: Pipeline<Either<L, R>>) -> (Pipeline<L>, Pipeline<R>) {
        // New channels for left or right output
        let (left_i, left_o) = Pipeline::new();
        let (right_i, right_o) = Pipeline::new();

        // Worker thread that feeds output from the given Receiver into either of the output channels
        thread::spawn(move || {
            // This loop goes on as long as the Sender corresponding to the
            // given receiver isn't closed
            for x in self.iter() {
                // Check whether or not an error has occurred on the channel
                match x {
                    Ok(val) =>
                        match val {
                            Either::Left(l) => { left_i.send(Ok(l)).unwrap(); },
                            Either::Right(r) => { right_i.send(Ok(r)).unwrap(); }
                        },
                    Err(err) => {
                        // Propagate any errors
                        left_i.send(Err(err.clone())).unwrap();
                        right_i.send(Err(err)).unwrap();
                        break;
                    }
                }
            }
            info!("Selection finished because of closed channel");
        });

        // Both of the output channels' Receivers are returned for further propagation
        (left_o, right_o)
    }

}


// Module containing pipeline tests
#[cfg(test)]
mod tests {
    use std::sync::mpsc::TryRecvError;
    use super::{Pipeline, Result, Error, ErrorKind};
    use super::either::Either;

    // Tests the from_value function
    #[test]
    fn test_from_value() {

        let o = Pipeline::<i64>::from_value(3);

        // The output channel should contain the object that from_value was called with
        assert_eq!(o.recv().unwrap().unwrap(), 3);
    }

    // Tests the from_vec function
    #[test]
    fn test_from_vec() {
        let o = Pipeline::<i64>::from_vec(vec![3, 4]);

        // The output channel should contain all elements in the Vec that from_vec was called with
        assert_eq!(o.recv().unwrap().unwrap(), 3);
        assert_eq!(o.recv().unwrap().unwrap(), 4);
    }

    // Tests the from_vec_sync function
    #[test]
    fn test_from_vec_sync() {
        let o = Pipeline::<i64>::from_vec_sync(vec![3, 4]);

        // The output channel should contain all elements in the Vec that from_vec_sync was called with
        assert_eq!(o.recv().unwrap().unwrap(), 3);
        assert_eq!(o.recv().unwrap().unwrap(), 4);
    }

    // Tests the map function
    #[test]
    fn test_map() {
        let (i, pipeline) = Pipeline::new();
        let recv = pipeline.map(|x: i64| -> Result<i64> { Ok(x + 1) });

        i.send(Ok(3)).unwrap();

        drop(i);

        // The output channel should contain a message that consists of the
        // original input transformed by the given closure (increased by one).
        assert_eq!(recv.recv().unwrap().unwrap(), 4);

    }

    // Tests the map_with_ctx function
    #[test]
    fn test_map_with_ctx() {
        let (i, pipeline) = Pipeline::<i64>::new();
        let (mut f, recv) = pipeline.map_with_ctx(|val, ctx| {
            *ctx += 1;
            Ok(val + 1)
        }, 0);

        i.send(Ok(2)).unwrap();

        drop(i);

        // The output channel must receive input incremented by 1
        assert_eq!(recv.recv().unwrap().unwrap(), 3);
        // The shared context must contain the amount of values that has
        // passed through the pipeline
        assert_eq!(f.get(), 1);
    }

    // Tests the each function
    #[test]
    fn test_each() {
        let (i1, pipeline1) = Pipeline::<i64>::new();
        let (i2, pipeline2) = Pipeline::<i64>::new();

        pipeline1.each(move |value| {
            i2.send(value).unwrap();
        });

        i1.send(Ok(3)).unwrap();

        drop(i1);

        // The testing output channel must receive all pipeline input
        assert_eq!(pipeline2.recv().unwrap(), Ok(3));
    }

    // Tests the each_with_ctx function
    #[test]
    fn test_each_with_ctx() {
        let (i1, pipeline1) = Pipeline::<i64>::new();
        let (i2, pipeline2) = Pipeline::<i64>::new();

        let mut f = pipeline1.each_with_ctx(move |val, ctx| {
            *ctx += 1;
            i2.send(val).unwrap();
        }, 0);

        i1.send(Ok(2)).unwrap();

        drop(i1);

        // The testing output channel must receive all pipeline input
        assert_eq!(pipeline2.recv().unwrap(), Ok(2));
        // The shared context must contain the amount of values that has
        // passed through the pipeline
        assert_eq!(f.get(), 1);
    }

    // Tests the merge function
    #[test]
    fn test_merge() {
        let (i1, pipeline1) = Pipeline::new();
        let (i2, pipeline2) = Pipeline::new();
        let pipeline = pipeline1.merge(pipeline2).map(|x: i64| -> Result<i64> { Ok(x + 1) });

        i1.send(Ok(3)).unwrap();
        i2.send(Ok(4)).unwrap();

        drop(i1);
        drop(i2);

        // The output channel should receive the message that was sent to
        // the first input channel.
        assert_eq!(pipeline.recv().unwrap().unwrap(), 4);
        // The output channel should receive the message that was sent to
        // the second input channel.
        assert_eq!(pipeline.recv().unwrap().unwrap(), 5);
    }

    // Tests the merge_either function
    #[test]
    fn test_merge_either() {
        let (i1, pipeline1) = Pipeline::<i64>::new();
        let (i2, pipeline2) = Pipeline::<i64>::new();
        let p = pipeline1.merge_either(pipeline2);

        i1.send(Ok(3)).unwrap();
        i2.send(Ok(4)).unwrap();

        drop(i1);
        drop(i2);

        // The output channel should receive the message that was sent to
        // the first input channel, wrapped in a Left
        assert_eq!(p.recv().unwrap().unwrap().left().unwrap(), 3);
        // The output channel should receive the message that was sent to
        // the second input channel, wrapped in a Right
        assert_eq!(p.recv().unwrap().unwrap().right().unwrap(), 4);
    }

    // Tests the duplicate function
    #[test]
    fn test_duplicate() {
        let (i, pipeline) = Pipeline::<i64>::new();
        let (p1, p2) = pipeline.duplicate();

        i.send(Ok(2)).unwrap();

        drop(i);

        // Both output channels should receive the message that was sent to
        // the input channel
        assert_eq!(p1.recv().unwrap().unwrap(), 2);
        assert_eq!(p2.recv().unwrap().unwrap(), 2);

    }

    // Tests the split function
    #[test]
    fn test_split_by() {
        let (i, pipeline) = Pipeline::new();
        let (left, right) = pipeline.split_by(|x: &i64| -> bool {
            *x > 3
        });

        i.send(Ok(2)).unwrap();
        i.send(Ok(4)).unwrap();

        drop(i);

        // The left output channel should contain the sent message
        // that did fulfill the predicate criteria (> 3)
        assert_eq!(left.recv().unwrap().unwrap(), 4);
        // The right output channel should contain the sent message
        // that did not fulfill the predicate criteria (!(> 3))
        assert_eq!(right.recv().unwrap().unwrap(), 2);
    }

    // Tests the split_by_either function
    #[test]
    fn test_split_by_either_stage() {
        let (i, pipeline) = Pipeline::<Either<i64, i64>>::new();
        let (left, right) = pipeline.split_by_either();

        i.send(Ok(Either::Left(3))).unwrap();
        i.send(Ok(Either::Right(4))).unwrap();

        drop(i);

        // The left output channel should contain the sent message
        // that is wrapped in a Left
        assert_eq!(left.recv().unwrap().unwrap(), 3);
        // The right output channel should contain the sent message
        // that is wrapped in a Right
        assert_eq!(right.recv().unwrap().unwrap(), 4);

    }

    // Tests the filter_ok function
    #[test]
    fn test_filter_ok() {
        let (i, pipeline) = Pipeline::<i64>::new();

        let recv = pipeline.filter_ok();

        i.send(Ok(3)).unwrap();
        i.send(Err(Error { kind: ErrorKind::Other, desc: "Test" })).unwrap();

        assert_eq!(recv.recv().unwrap(), 3);
        assert!(recv.try_recv().is_err());
        assert_eq!(recv.try_recv().err().unwrap(), TryRecvError::Empty);
    }

    // Tests the filter_err function
    #[test]
    fn test_filter_err() {
        let (i, pipeline) = Pipeline::<i64>::new();

        let recv = pipeline.filter_err();

        i.send(Ok(3)).unwrap();
        i.send(Err(Error { kind: ErrorKind::Other, desc: "Test" })).unwrap();

        assert_eq!(recv.recv().unwrap(), Error { kind: ErrorKind::Other, desc: "Test" });
        assert!(recv.try_recv().is_err());
        assert_eq!(recv.try_recv().err().unwrap(), TryRecvError::Empty);
    }

    // Tests the collect_ok function
    #[test]
    fn test_collect_ok() {
        let (i, pipeline) = Pipeline::<i64>::new();

        i.send(Ok(3)).unwrap();
        i.send(Err(Error { kind: ErrorKind::Other, desc: "Test" })).unwrap();

        drop(i);

        let vec = pipeline.collect_ok();

        assert_eq!(vec, vec![3]);
    }

    // Tests the collect_err function
    #[test]
    fn test_collect_err() {
        let (i, pipeline) = Pipeline::<i64>::new();

        i.send(Ok(3)).unwrap();
        i.send(Err(Error { kind: ErrorKind::Other, desc: "Test" })).unwrap();

        drop(i);

        let vec = pipeline.collect_err();

        assert_eq!(vec, vec![Error { kind: ErrorKind::Other, desc: "Test" }]);
    }


}

//! # The either module
//! This module contains the Either type and implementations


/// Either is a type that represents either of two types
///
/// It is split up into a right and a left value, similar
/// to how the Result type functions.
pub enum Either<L, R> {
    Left(L),
    Right(R)
}

impl <L, R> Either<L, R> {

    /// Returns true if the Either contains an element of type L
    pub fn is_left(&self) -> bool {
        match *self {
            Either::Left(_) => true,
            Either::Right(_) => false
        }
    }

    /// Returns true if the Either contains an element of type R
    pub fn is_right(&self) -> bool {
        match *self {
            Either::Left(_) => false,
            Either::Right(_) => true
        }
    }

    /// Converts the Either into an Option, returning Some(L) or None
    pub fn left(self) -> Option<L> {
        match self {
            Either::Left(val) => Some(val),
            Either::Right(_) => None
        }
    }

    /// Converts the Either into an Option, returning Some(R) or None
    pub fn right(self) -> Option<R> {
        match self {
            Either::Left(_) => None,
            Either::Right(val) => Some(val)
        }
    }
}

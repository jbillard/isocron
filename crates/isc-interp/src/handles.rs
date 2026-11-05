//! The resources of the system in the interpreter (D339): a connection or
//! a listener of TCP, held in a cell that `close` empties; the resource is
//! given back to the system when it is closed or when its last owner goes,
//! as the compiled program does when it frees the value.

use std::cell::RefCell;
use std::net::{TcpListener, TcpStream};

#[derive(Debug)]
pub enum Resource {
    Conn(TcpStream),
    Listener(TcpListener),
    /// A listener and the connections it accepted (D342), all non-blocking,
    /// by number in the order they came (a number never given twice).
    Server { listener: TcpListener, next: i64, conns: Vec<(i64, TcpStream)> },
}

#[derive(Debug, Default)]
pub struct Cell(pub RefCell<Option<Resource>>);

impl PartialEq for Cell {
    fn eq(&self, other: &Cell) -> bool {
        std::ptr::eq(self, other)
    }
}

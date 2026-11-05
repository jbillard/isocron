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
}

#[derive(Debug, Default)]
pub struct Cell(pub RefCell<Option<Resource>>);

impl PartialEq for Cell {
    fn eq(&self, other: &Cell) -> bool {
        std::ptr::eq(self, other)
    }
}

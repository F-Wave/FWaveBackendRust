use std::sync::{atomic, Arc, Mutex};
use std::cell::UnsafeCell;
use std::num::Wrapping;
use std::sync::atomic::{Ordering, AtomicBool};
use std::task::{Waker, Poll};
use crate::spinlock::Spinlock;

/*
struct State {
    head: u16,
    tail: u16,
}*/

/*
struct Waitlist<T: Send> {
    receivers: Vec<Waker>,
    senders: Vec<(T, Waker)>,
}
*/

struct Internal<T: Send> {
    queue: Vec<Option<T>>, //ring buffer
    receivers: Vec<Waker>,
    senders: Vec<(T, Waker)>,
    head: Wrapping<u32>,
    tail: Wrapping<u32>,
    /*read_at: atomic::AtomicU32,
    head: atomic::AtomicU32,
    write_at: atomic::AtomicU32,
    tail: atomic::AtomicU32,*/
}
/*
fn head(state: u32) -> u16 {
    (state & (2 << 8 - 1)) as u16
}

fn tail(state: u32) -> u16  {
    (state >> 8) as u16
}

fn get_head_and_tail(state: u32) -> (Wrapping<u16>, Wrapping<u16>) {
    (head(state), tail(state))
}

fn from_head_and_tail(head: u16, tail: u16) -> u32 {
    head as u32 | ((tail as u32) << 8)
}*/

pub struct Receiver<T: Send> {
    internal: Arc<Spinlock<Internal<T>>>,
}

pub struct Sender<T: Send> {
    internal: Arc<Spinlock<Internal<T>>>
}

unsafe impl<T: Send> Sync for Receiver<T> {}
unsafe impl<T: Send> Sync for Sender<T> {}
unsafe impl<T: Send> Send for Receiver<T> {}
unsafe impl<T: Send> Send for Sender<T> {}

impl<T: Send> Clone for Sender<T> {
    fn clone(&self) -> Sender<T> {
        Sender{ internal: self.internal.clone() }
    }
}

impl<T: Send> Sender<T> {
    fn send_or_add_to_waitlist(&mut self, waker: Option<Waker>, value: T) -> bool {
        let mut internal = self.internal.lock();

        let n = internal.queue.len() as u32; //should never resize

        let is_full = (internal.head - internal.tail).0 == n;
        debug_assert!((internal.head - internal.tail).0 <= n);

        if is_full {
            if let Some(waker) = waker {
                internal.senders.push((value, waker));
            }
            return false
        }

        debug_assert!(internal.queue[internal.head.0 as usize].is_none());

        let head = internal.head;
        internal.queue[(head.0 % n) as usize] = Some(value);
        internal.head += Wrapping(1);

        if let Some(waker) = internal.receivers.pop() {
            waker.wake()
        }

        return true
    }

    pub fn try_send(&mut self, value: T) -> bool { //returns true if sent succeded
        self.send_or_add_to_waitlist(None, value)
    }

    pub async fn send(&mut self, value: T) {
        //let first = AtomicBool::new(true);

        let mut first = Some(value);

        tokio::future::poll_fn(|ctx| {
            let value = match first.take() {
                Some(value) => value,
                None => return Poll::Ready(())
            };

            if self.send_or_add_to_waitlist(Some(ctx.waker().clone()), value) {
                Poll::Ready(())
            } else {
                Poll::Pending //check if executed twice
            }
        }).await
    }
}


impl<T: Send> Clone for Receiver<T> {
    fn clone(&self) -> Receiver<T> {
        Receiver{ internal: self.internal.clone() }
    }
}

impl<T: Send> Receiver<T> {
    pub fn recv_or_add_to_waitlist(&mut self, waker: Option<Waker>) -> Option<T> {
        let mut internal = self.internal.lock();
        let n = internal.queue.len() as u32; //should never resize

        let is_empty = (internal.head - internal.tail).0 == 0;
        if is_empty {
            {
                if let Some((value, waker)) = internal.senders.pop() {
                    waker.wake();
                    return Some(value)
                }
            }

            if let Some(waker) = waker {
                internal.receivers.push(waker);
            }

            return None
        }

        let tail = internal.tail;
        let value = internal.queue[(tail.0 % n) as usize].take().unwrap();
        internal.tail += Wrapping(1);

        Some(value)
    }

    pub async fn recv(&mut self) -> T {
        tokio::future::poll_fn(|ctx| {
            match self.recv_or_add_to_waitlist(Some(ctx.waker().clone())) {
                Some(value) => Poll::Ready(value),
                None => Poll::Pending
            }
        }).await
    }
}

pub fn channel<T: Send>(bounded: usize) -> (Sender<T>, Receiver<T>) {
    let internal = Arc::new(Spinlock::new(Internal{
        queue: (0..bounded).map(|_| None).collect(),
        //write_at: atomic::AtomicU32::new(0),
        //read_at: atomic::AtomicU32::new(0),
        receivers: vec![],
        senders: vec![],
        tail: Wrapping(0),
        head: Wrapping(0),
    }));

    (Sender{internal: internal.clone()}, Receiver{internal: internal.clone()})
}
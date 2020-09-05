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
    receivers: Spinlock<Vec<Waker>>,
    senders: Spinlock<Vec<(T, Waker)>>,
    read_at: atomic::AtomicU32,
    head: atomic::AtomicU32,
    write_at: atomic::AtomicU32,
    tail: atomic::AtomicU32,
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
    internal: Arc<UnsafeCell<Internal<T>>>,
}

pub struct Sender<T: Send> {
    internal: Arc<UnsafeCell<Internal<T>>>
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
        let internal: &mut Internal<T> = unsafe { &mut *self.internal.get() };

        let n = internal.queue.len() as u32; //should never resize
        let mut write_at = Wrapping(0);

        loop {
            write_at = Wrapping(internal.write_at.load(Ordering::Acquire)); //check ordering
            let tail = Wrapping(internal.tail.load(Ordering::Acquire));

            let is_full = (write_at - tail).0 == n;
            assert!((write_at - tail).0 <= n);

            if is_full {
                if let Some(waker) = waker {
                    let mut senders = internal.senders.lock();
                    senders.push((value, waker));
                }
                return false
            }

            let found = internal.write_at.compare_and_swap(write_at.0, (write_at + Wrapping(1)).0, Ordering::Release);
            if found == write_at.0 { break }
        }

        assert!(internal.queue[(write_at.0 % n) as usize].is_none());
        internal.queue[(write_at.0 % n) as usize] = Some(value);

        while write_at.0 != internal.head.compare_and_swap(write_at.0, (write_at + Wrapping(1)).0, Ordering::Release) {
            //ensures that receiver only sees elements which are written to
            //as in let's say that two threads are granted write access, elements 1 and 2, write to element 2 completes and write_at is set to 2
            //which will cause a data race since the write to element 1 hasn't completed yet
        }

        let mut receivers = internal.receivers.lock();
        if let Some(waker) = receivers.pop() {
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
        let internal: &mut Internal<T> = unsafe { &mut *self.internal.get() };
        let n = internal.queue.len() as u32; //should never resize
        let mut read_at = Wrapping(0);

        loop {
            read_at = Wrapping(internal.read_at.load(Ordering::Acquire)); //check ordering
            let head = Wrapping(internal.head.load(Ordering::Acquire));

            let is_empty = (head - read_at).0 == 0;
            if is_empty {
                {
                    let mut senders = internal.senders.lock();
                    if let Some((value, waker)) = senders.pop() {
                        waker.wake();
                        return Some(value)
                    }
                }

                if let Some(waker) = waker {
                    let mut receivers = internal.receivers.lock();
                    receivers.push(waker);
                }

                return None
            }

            let found = internal.read_at.compare_and_swap(read_at.0, (read_at + Wrapping(1)).0, Ordering::Release);
            if found == read_at.0 { break }
        }

        let value = internal.queue[(read_at.0 % n) as usize].take().unwrap();

        while read_at.0 != internal.tail.compare_and_swap(read_at.0, (read_at + Wrapping(1)).0, Ordering::Release) {
            //ensures that the sender does not write over values currently being read
        }

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
    let internal = Arc::new(UnsafeCell::new(Internal{
        queue: (0..bounded).map(|_| None).collect(),
        write_at: atomic::AtomicU32::new(0),
        read_at: atomic::AtomicU32::new(0),
        receivers: Spinlock::new(vec![]),
        senders: Spinlock::new(vec![]),
        tail: atomic::AtomicU32::new(0),
        head: atomic::AtomicU32::new(0),
    }));

    (Sender{internal: internal.clone()}, Receiver{internal: internal.clone()})
}
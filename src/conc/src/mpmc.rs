use std::sync::{atomic, Arc, Mutex};
use std::cell::UnsafeCell;
use std::num::Wrapping;
use std::sync::atomic::{Ordering, AtomicBool};
use std::task::{Waker, Poll};
use crate::spinlock::Spinlock;
use std::mem;
use std::net::Shutdown::Write;
use std::time::Duration;

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

struct Tricolor<T: Send> {
    color: atomic::AtomicU8,
    value: UnsafeCell<Option<T>>,
}

unsafe impl<T: Send> Sync for Tricolor<T> {}
unsafe impl<T: Send> Send for Tricolor<T> {}

const EMPTY : u8 = 0;
const WRITTEN_TO : u8 = 1;
const SAFE_TO_READ : u8 = 2;
const READING_FROM : u8 = 3;

fn cas_safe_to_read(color: &atomic::AtomicU8) -> bool {
    loop {
        let previous = color.compare_and_swap(SAFE_TO_READ, READING_FROM, Ordering::Acquire);
        if previous == EMPTY {
            return false
        }
        else if previous == SAFE_TO_READ {
            break;
        }
    }

    true
}

impl<T: Send> Tricolor<T> {
    fn new() -> Tricolor<T> {
        Tricolor {
            color: atomic::AtomicU8::new(0),
            value: UnsafeCell::new(None)
        }
    }

    fn is_empty(&self) -> bool {
        self.color.load(Ordering::Acquire) == EMPTY
    }

    fn acquire(&self, new: u8) -> &mut T {
        loop {
            let color = self.color.load(Ordering::Relaxed);
            let previous = if color == EMPTY { EMPTY } else { SAFE_TO_READ };

            if previous == self.color.compare_and_swap(previous, new, Ordering::Acquire) {
                break
            }
        }
        unsafe { (*self.value.get()).as_mut().unwrap() }
    }

    fn release(&self, empty: bool) {
        self.color.store(if empty { EMPTY } else { SAFE_TO_READ }, Ordering::Release);
    }

    fn write(&self, value: T) { //this will block untill the value has been read
        while self.color.compare_and_swap(EMPTY, WRITTEN_TO, Ordering::Acquire) != EMPTY {}
        unsafe {
            (*self.value.get()) = Some(value);
        }
        self.color.store(SAFE_TO_READ, Ordering::Release);
    }

    unsafe fn take(&self) -> T {
        let cell = unsafe { &mut *self.value.get() };
        cell.take().unwrap()
        //mem::replace(cell, None.assume_init()
    }

    fn read(&self) -> Option<T> {
        if !cas_safe_to_read(&self.color) {
            return None
        }

        let value = unsafe { self.take() };

        self.color.store(EMPTY, Ordering::Release);

        Some(value)
    }
}

impl<T: Send> Drop for Tricolor<T> {
    fn drop(&mut self) {
        loop {
            let previous = self.color.compare_and_swap(SAFE_TO_READ, READING_FROM, Ordering::Acquire);
            if previous == EMPTY { return }
            if previous == SAFE_TO_READ { break }
        }

        unsafe { self.take() };
    }
}

struct TricolorVec<T: Send> {
    color: atomic::AtomicU8,
    value: UnsafeCell<Vec<T>>,
}

unsafe impl<T: Send> Sync for TricolorVec<T> {}
unsafe impl<T: Send> Send for TricolorVec<T> {}

impl<T: Send> TricolorVec<T> {
    fn new() -> TricolorVec<T> {
        TricolorVec{
            color: atomic::AtomicU8::new(EMPTY),
            value: UnsafeCell::new(Vec::new())
        }
    }

    fn acquire(&self, new: u8) {
        loop {
            let color = self.color.load(Ordering::Relaxed);
            let previous = if color == EMPTY { EMPTY } else { SAFE_TO_READ };

            if previous == self.color.compare_and_swap(previous, new, Ordering::Acquire) {
                break
            }
        }
    }

    fn release(&self, empty: bool) {
        if empty {
            self.color.store(EMPTY, Ordering::Release);
        } else {
            self.color.store(SAFE_TO_READ, Ordering::Release);
        }
    }

    unsafe fn get_vec(&self) -> &mut Vec<T> {
        &mut *self.value.get()
    }

    fn pop(&self) -> Option<T> {
        if !cas_safe_to_read(&self.color) {
            return None
        }

        let vec = unsafe { self.get_vec() };
        let value = vec.pop().unwrap();

        self.release(vec.len() == 0);
        Some(value)
    }

    fn push(&self, value: T) {
        self.acquire(WRITTEN_TO);

        let vec = unsafe { &mut *self.value.get() };
        vec.push(value);

        self.release(false);
    }
}

pub struct Waitlist<T> {
    pub receivers: Vec<Waker>,
    pub senders: Vec<(T, Waker)>,
}

pub struct Internal<T: Send> {
    pub queue: Vec<Option<T>>, //ring buffer
    //pub waitlist: Waitlist<T>,
    pub receivers: Vec<Waker>,
    pub senders: Vec<(T, Waker)>,
    pub head: Wrapping<u32>,
    pub tail: Wrapping<u32>,
    pub mask: u32,
    /*read_at: atomic::AtomicU32,
    head: atomic::AtomicU32,
    write_at: atomic::AtomicU32,
    tail: atomic::AtomicU32,*/
}

unsafe impl<T: Send> Sync for Internal<T> {}
unsafe impl<T: Send> Send for Internal<T> {}

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
    pub internal: Arc<Mutex<Internal<T>>>,
}

pub struct Sender<T: Send> {
    internal: Arc<Mutex<Internal<T>>>
}

impl<T: Send> Clone for Sender<T> {
    fn clone(&self) -> Sender<T> {
        Sender{ internal: self.internal.clone() }
    }
}

impl<T: Send> Sender<T> {
    fn send_or_add_to_waitlist(&self, waker: Option<Waker>, value: T) -> bool {
        let mut internal = self.internal.as_ref().lock().unwrap();
        let n = internal.queue.len() as u32; //should never resize

        let is_full = (internal.head - internal.tail).0 >= n;

        if is_full {
            if let Some(waker) = waker {
                internal.senders.push((value, waker)); //remove clone somehow

                if let Some(waker) = internal.receivers.pop() {
                    waker.wake();
                }

                return false
            }
        }

        let head = (internal.head.0 & internal.mask) as usize;
        internal.queue[head] = Some(value);
        //println!("Writing at {}", head);
        internal.head += Wrapping(1);

        //println!("Inserted at {}", head);

        if let Some(waker) = internal.receivers.pop() {
            waker.wake();
        }

        return true
    }

    pub fn try_send(&mut self, value: T) -> bool { //returns true if sent succeded
        self.send_or_add_to_waitlist(None, value)
    }

    pub async fn send(&self, value: T) {
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
    pub fn recv_or_add_to_waitlist(&self, waker: Option<Waker>) -> Option<T> {
        let mut internal = self.internal.as_ref().lock().unwrap();
        let n = internal.queue.len() as u32; //should never resize

        let is_empty = internal.head == internal.tail; // (head - tail).0 == 0;
        if is_empty {
            if let Some((value, waker)) = internal.senders.pop() {
                waker.wake();
                return Some(value)
            }

            if let Some(waker) = waker {
                internal.receivers.push(waker);
            }

            return None
        }

        let tail = (internal.tail.0 & internal.mask) as usize;
        let value = internal.queue[tail].take();
        //println!("Reading at {}", tail);
        internal.tail += Wrapping(1);

        match value {
            Some(value) => Some(value),
            None => {
                panic!("Should never happen!!!!!");
            }
        }
    }

    pub async fn recv(&self) -> T {
        tokio::future::poll_fn(|ctx| {
            match self.recv_or_add_to_waitlist(Some(ctx.waker().clone())) {
                Some(value) => Poll::Ready(value),
                None => Poll::Pending
            }
        }).await
    }
}

pub fn channel<T: Send>(bounded: u32) -> (Sender<T>, Receiver<T>) {
    /*let tricolor = Tricolor::new();
    unsafe { *tricolor.value.get() =
        Some(Waitlist{
            receivers: vec![],
            senders: vec![],
        });
    }*/

    if bounded != 1 && (bounded & (bounded - 1)) != 0 {
        panic!("Channel buffer size must be a power of 2");
    }

    let internal = Arc::new(Mutex::new(Internal{
        queue: (0..bounded).map(|_| None).collect(),
        senders: vec![],
        receivers: vec![],
        tail: Wrapping(0),
        head: Wrapping(0),
        mask: bounded - 1
    }));

    (Sender{internal: internal.clone()}, Receiver{internal: internal.clone()})
}
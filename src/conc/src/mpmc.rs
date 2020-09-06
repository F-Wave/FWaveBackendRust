use std::sync::{atomic, Arc, Mutex};
use std::cell::UnsafeCell;
use std::num::Wrapping;
use std::sync::atomic::{Ordering, AtomicBool};
use std::task::{Waker, Poll};
use crate::spinlock::Spinlock;
use std::mem;

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
    value: UnsafeCell<mem::MaybeUninit<T>>,
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
            value: UnsafeCell::new(unsafe { mem::MaybeUninit::uninit() })
        }
    }

    fn write(&self, value: T) { //this will block untill the value has been read
        while self.color.compare_and_swap(EMPTY, WRITTEN_TO, Ordering::Acquire) != EMPTY {}
        unsafe {
            (*self.value.get()).as_mut_ptr().write(value);
        }
        self.color.store(SAFE_TO_READ, Ordering::Release);
    }

    unsafe fn take(&self) -> T {
        let cell = unsafe { &mut *self.value.get() };
        mem::replace(cell, mem::MaybeUninit::uninit()).assume_init()
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

struct Internal<T: Send> {
    queue: Vec<Tricolor<T>>, //ring buffer
    receivers: TricolorVec<Waker>,
    senders: TricolorVec<(T, Waker)>,
    head: atomic::AtomicU32,
    tail: atomic::AtomicU32,
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
    internal: Arc<Internal<T>>,
}

pub struct Sender<T: Send> {
    internal: Arc<Internal<T>>
}

impl<T: Send> Clone for Sender<T> {
    fn clone(&self) -> Sender<T> {
        Sender{ internal: self.internal.clone() }
    }
}

impl<T: Send> Sender<T> {
    fn send_or_add_to_waitlist(&self, waker: Option<Waker>, value: T) -> bool {
        let internal = self.internal.as_ref();

        let n = internal.queue.len() as u32; //should never resize
        let mut insert_at : usize = 0;

        loop {
            let head = Wrapping(internal.head.load(Ordering::Acquire));
            let tail = Wrapping(internal.tail.load(Ordering::Acquire));

            let is_full = (head - tail).0 == n;
            debug_assert!((head - tail).0 <= n);

            if is_full {
                if let Some(waker) = waker {
                    internal.senders.push((value, waker));
                }
                return false
            }

            if head.0 == internal.head.compare_and_swap(head.0, (head + Wrapping(1)).0, Ordering::Release) {
                insert_at = head.0 as usize;
                break;
            }
        }

        internal.queue[insert_at % n as usize].write(value);

        if let Some(waker) = internal.receivers.pop() {
            waker.wake()
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
        let internal = self.internal.as_ref();
        let n = internal.queue.len() as u32; //should never resize
        let mut read_at: usize = 0;

        loop {
            let head = Wrapping(internal.head.load(Ordering::Acquire));
            let tail = Wrapping(internal.tail.load(Ordering::Acquire));

            let is_empty = (head - tail).0 == 0;
            if is_empty {
                // -- inserted value - race condition
                //println!("is empty!");

                match &waker {
                    Some(waker) => {
                        if let Some((value, waker)) = internal.senders.pop() {
                            waker.wake();
                            return Some(value)
                        }

                        // -- inserted value - race condition

                        internal.receivers.push(waker.clone());

                        /*internal.senders.acquire(READING_FROM);

                        let senders = unsafe { internal.senders.get_vec() };

                        let is_empty = (head - tail).0 == 0;
                        if !is_empty {
                            internal.senders.release(senders.len() == 0);
                            continue;
                        }


                        if let Some((value, waker)) = senders.pop() {
                            waker.wake();
                            internal.senders.release(senders.len() == 0);
                            return Some(value)
                        }

                        internal.receivers.push(waker.clone());
                        internal.senders.release(senders.len() == 0);*/
                    },
                    None => {
                        if let Some((value, waker)) = internal.senders.pop() {
                            waker.wake();
                            return Some(value)
                        }
                    }
                }

                return None
            }

            if tail.0 == internal.tail.compare_and_swap(tail.0, (tail + Wrapping(1)).0, Ordering::Release) {
                read_at = tail.0 as usize;
                break;
            }
        }

        loop {
            if let Some(value) = internal.queue[read_at % n as usize].read() {
                return Some(value)
            }
        }

        None

        //println!("Read at {}", read_at);
        //internal.queue[read_at % n as usize].read()
        //Some(internal.queue[read_at % n as usize].read().unwrap())
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

pub fn channel<T: Send>(bounded: usize) -> (Sender<T>, Receiver<T>) {
    let internal = Arc::new(Internal{
        queue: (0..bounded).map(|_| Tricolor::new()).collect(),
        receivers: TricolorVec::new(),
        senders: TricolorVec::new(),
        tail: atomic::AtomicU32::new(0),
        head: atomic::AtomicU32::new(0),
    });

    (Sender{internal: internal.clone()}, Receiver{internal: internal.clone()})
}
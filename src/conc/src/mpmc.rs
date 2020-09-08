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
    pub queue: Vec<(atomic::AtomicU32, UnsafeCell<Option<T>>)>, //ring buffer
    pub waitlist: Spinlock<Waitlist<T>>,
    pub head: atomic::AtomicU64,
    pub tail: atomic::AtomicU64,
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
    pub internal: Arc<Internal<T>>,
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
        let mut insert_at : u32 = 0;
        let mut next_head : u32 = 0;
        let mut lap : u32 = 0;

        let mut waitlist = internal.waitlist.lock();

        //println!("=== SEND ATTEMPT ===");

        loop {
            let head_and_lap = internal.head.load(Ordering::Acquire);
            lap = (head_and_lap >> 32) as u32;
            let head = head_and_lap as u32;
            let elap = internal.queue[head as usize].0.load(Ordering::Acquire);
            //let tail = Wrapping(internal.tail.load(Ordering::Acquire));

            let is_full = (lap as i32 - elap as i32) > 0;//  !internal.queue[(head % n) as usize].is_empty();

            /*if (head - tail).0 > n {
                println!("Should not happen!!! {} {}", head, tail);
                continue
            }*/

            if is_full {
                if let Some(waker) = &waker {
                    //let mut waitlist = internal.waitlist.lock(); //internal.waitlist.acquire(WRITTEN_TO);

                    let head_and_lap = internal.head.load(Ordering::Acquire);
                    let lap = (head_and_lap >> 32) as u32;
                    let head = head_and_lap as u32;
                    let elap = internal.queue[head as usize].0.load(Ordering::Acquire);

                    if lap == elap { //inserted value which has not been read yet
                        //internal.waitlist.release(waitlist.receivers.len() == 0);
                        continue
                    }


                    waitlist.senders.push((value, waker.clone())); //remove clone somehow

                    if let Some(waker) = waitlist.receivers.pop() {
                        //println!("Removing receiver from waitlist rare!!!! {}", head_and_lap);
                        waker.wake();
                    }

                    //internal.waitlist.release(waitlist.receivers.len() == 0);
                    //println!("Adding sender in waitlist");
                }
                return false
            }
            else if elap == lap {
                let next_head =
                    if head + 1 < n { head_and_lap + 1 } //todo add wrapping!
                    else { ((lap + 2) as u64) << 32 };

                if head_and_lap == internal.head.compare_and_swap(head_and_lap, next_head, Ordering::Release) {
                    insert_at = head;
                    break;
                }
            }


        }

        /*
        waitlist.receivers.push(waker);
        */


        //println!("Writing at {}, lap {}", insert_at as usize, lap);


        unsafe { *internal.queue[insert_at as usize].1.get() = Some(value) };
        internal.queue[insert_at as usize].0.store(lap + 1, Ordering::Release);

        if let Some(waker) = waitlist.receivers.pop() {
            waker.wake();
        }

        /*
        loop {
            let previous = internal.waitlist.color.compare_and_swap(SAFE_TO_READ, READING_FROM, Ordering::AcqRel);
            if previous == EMPTY {
                return true
            }
            if previous == SAFE_TO_READ {
                let waitlist = unsafe { (*internal.waitlist.value.get()).as_mut().unwrap() };
                let waker = waitlist.receivers.pop().unwrap();
                waker.wake();

                println!("Removed receiver from waitlist {}", waitlist.receivers.len());
                internal.waitlist.release(waitlist.receivers.len() == 0);

                break;
            }
        };*/

        //println!("Wrote value {}", internal.receivers.color.load(Ordering::Acquire));

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
        let mut read_at: u32 = 0;
        let mut next_read_at: u64 = 0;
        let mut lap : u32 = 0;

        loop {
            let tail_and_lap = internal.tail.load(Ordering::Relaxed);
            lap = (tail_and_lap >> 32) as u32;
            let tail = tail_and_lap as u32;
            let elap = internal.queue[tail as usize].0.load(Ordering::Relaxed);

            let is_empty = lap as i32 - elap as i32 > 0; // (head - tail).0 == 0;
            if is_empty {
                let mut waitlist = internal.waitlist.lock(); //internal.waitlist.acquire(WRITTEN_TO);

                let tail_and_lap = internal.tail.load(Ordering::Relaxed);
                lap = (tail_and_lap >> 32) as u32;
                let tail = tail_and_lap as u32;
                let elap = internal.queue[tail as usize].0.load(Ordering::Acquire);

                if lap == elap {
                    continue;
                }

                if let Some((value, waker)) = waitlist.senders.pop() {
                    waker.wake();
                    return Some(value)
                }

                if let Some(waker) = waker {
                    waitlist.receivers.push(waker);
                }

                return None
            }
            else if lap == elap {
                let next_tail =
                    if tail + 1 < n { tail_and_lap + 1 }
                    else { ((lap + 2) as u64) << 32 };

                if tail_and_lap != internal.tail.compare_and_swap(tail_and_lap, next_tail, Ordering::Relaxed) { continue }

                read_at = tail;
                next_read_at = next_tail;
                break;

            }
        }

        //std::thread::sleep(Duration::from_millis(1));

        //println!("Reading at {}, lap: {}, next_tail: {}", read_at, lap, next_read_at);

        let value = unsafe { &mut *internal.queue[read_at as usize].1.get() }.take().unwrap();
        internal.queue[read_at as usize].0.store(lap + 1, Ordering::Release);


        //panic!("Failed to read {} in time {}", read_at % n, read_at);

        Some(value)
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
    /*let tricolor = Tricolor::new();
    unsafe { *tricolor.value.get() =
        Some(Waitlist{
            receivers: vec![],
            senders: vec![],
        });
    }*/

    let internal = Arc::new(Internal{
        queue: (0..bounded).map(|_| (atomic::AtomicU32::new(0), UnsafeCell::new(None))).collect(),
        waitlist: Spinlock::new(Waitlist{senders: vec![], receivers: vec![]}),
        tail: atomic::AtomicU64::new(1u64 << 32),
        head: atomic::AtomicU64::new(0),
    });

    (Sender{internal: internal.clone()}, Receiver{internal: internal.clone()})
}
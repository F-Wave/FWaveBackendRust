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
/*
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
}*/

struct Waitlist<T> {
    receivers: Vec<Waker>,
    senders: Vec<(T, Waker)>
}

struct Internal<T: Send> {
    queue: UnsafeCell<Vec<(atomic::AtomicU32, Option<T>)>>, //ring buffer
    wait_list: Spinlock<Waitlist<T>>,
    receivers_empty: atomic::AtomicBool,
    head: atomic::AtomicU64,
    tail: atomic::AtomicU64,
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
    fn is_full(&self) -> bool {
        let internal = self.internal.as_ref();
        let queue = unsafe { &mut *internal.queue.get() };
        let head = internal.head.load(Ordering::Acquire);
        let pos = head as u32;
        let elap = queue[pos as usize].0.load(Ordering::Acquire);
        let lap: u32 = (head >> 32) as u32;

        (lap as i32 - elap as i32) > 0
    }

    fn send_or_add_to_waitlist(&self, waker: Option<Waker>, value: T) -> bool {
        let internal = self.internal.as_ref();
        let queue = unsafe { &mut *internal.queue.get() };

        let n = queue.len() as u32; //should never resize
        let mut insert_at : usize = 0;

        loop {
            let head = internal.head.load(Ordering::Acquire);
            let pos = head as u32;
            let elap = queue[pos as usize].0.load(Ordering::Acquire);
            let lap: u32 = (head >> 32) as u32;

            if lap == elap {
                let new_head =
                    if pos + 1 < n { head + 1 } else { (lap as u64 + 2) << 32 };



                if head != internal.head.compare_and_swap(head, new_head, Ordering::Release) { continue }
                // Do non-atomic write
                queue[pos as usize].1 = Some(value);
                // Make the element available for reading.

                //println!("===== Acquiring spin lock =====");
                queue[pos as usize].0.store(elap + 1, Ordering::Release);

                //println!("Write lap {}: {}", lap, pos);

                if !internal.receivers_empty.load(Ordering::Acquire) {
                    let mut waitlist = internal.wait_list.lock(); //hopefully this can be eliminated somehow

                    //println!("Write lap {}: {}", lap, pos);

                    if let Some(waker) = waitlist.receivers.pop() {
                        //println!("Woke up read");
                        waker.wake();
                    }

                    internal.receivers_empty.store(waitlist.receivers.len() == 0, Ordering::Release);
                } else {
                    //if internal.wait_list.lock().receivers.len()
                    //println!("receivers : {}", internal.wait_list.lock().receivers.len());
                    //println!("acording to bool : {}", internal.receivers_empty.load(Ordering::Acquire));
                }

                //println!("Release mutex!");

                return true
            } else if (lap as i32 - elap as i32) > 0 {
                // The element is not yet read on the previous lap,
                // the chan is full.
                if let Some(waker) = &waker {
                    let mut waitlist = internal.wait_list.lock();

                    if !self.is_full() {
                        continue
                    }

                    waitlist.senders.push((value, waker.clone()));
                }

                break
            } else {
                // The element has already been written on this lap,
                // this means that c->sendx has been changed as well,
                // retry.
            }
        }

        return false
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

            //println!("Attempting write");
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
    //todo remove duplication
    fn is_empty(&self) -> bool {
        let queue = unsafe { &mut *self.internal.queue.get() };
        let tail = self.internal.tail.load(Ordering::Acquire);
        let pos = tail as u32;
        let elap = queue[pos as usize].0.load(Ordering::Acquire);
        let lap: u32 = (tail >> 32) as u32;

        (lap as i32  - elap as i32 ) > 0
    }

    pub fn recv_or_add_to_waitlist(&self, waker: Option<Waker>) -> Option<T> {
        let internal = self.internal.as_ref();
        let queue = unsafe { &mut *internal.queue.get() };

        let n = queue.len() as u32; //should never resize
        let mut insert_at : usize = 0;

        loop {
            let tail = internal.tail.load(Ordering::Acquire);
            let pos = tail as u32;
            let elap = queue[pos as usize].0.load(Ordering::Acquire);
            let lap: u32 = (tail >> 32) as u32;

            if lap == elap {
                let new_tail =
                    if pos + 1 < n { tail + 1 } else { (lap as u64 + 2) << 32 };

                if tail != internal.tail.compare_and_swap(tail, new_tail, Ordering::Release) { continue }

                //println!("read lap {}, n: {}, next lap: {}", lap, pos, new_tail >> 32);
                // Do non-atomic read
                let value = queue[pos as usize].1.take().unwrap();
                // Make the element available for reading.
                queue[pos as usize].0.store(elap + 1, Ordering::Release);

                return Some(value)
            } else if (lap as i32  - elap as i32 ) > 0 {
                // The element is not yet read on the previous lap,
                // the chan is empty.

                let mut waitlist = internal.wait_list.lock();

                //println!("===== Acquired spinlock =====");
                if !self.is_empty() {
                    continue
                }

                //println!("Num senders: {}", waitlist.senders.len());

                if let Some((value, waker)) = waitlist.senders.pop() {
                    //println!("Woke up sender and read value!");
                    waker.wake();
                    return Some(value)
                }

                if let Some(waker) = waker {
                    internal.receivers_empty.store(false, Ordering::Release);
                    waitlist.receivers.push(waker);
                    //println!("Insert in read waitlist");
                }

                //println!("Released spinlock!");

                break;
            } else {
                //println!("Elap {}, n: {}", elap, pos);
                //println!("Else in read {}", (lap as i32  - elap as i32 ));

                //assert!((lap as i32  - elap as i32 ) > 0);
                // The element has already been read on this lap,
                // this means that c->sendx has been changed as well,
                // retry.
            }
        }


        return None
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
        queue: UnsafeCell::new((0..bounded).map(|_| (atomic::AtomicU32::new(0), None)).collect()),
        wait_list: Spinlock::new(Waitlist{
           senders: vec![], receivers: vec![],
        }),
        receivers_empty: atomic::AtomicBool::new(true),
        tail: atomic::AtomicU64::new(1u64 << 32),
        head: atomic::AtomicU64::new(0u64 << 32),
    });

    (Sender{internal: internal.clone()}, Receiver{internal: internal.clone()})
}
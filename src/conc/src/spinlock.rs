use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::ops::{DerefMut, Deref};

pub struct Spinlock<T: Send> {
    cell: UnsafeCell<T>,
    flag: AtomicBool
}

pub struct SpinlockGuard<'a, T: Send> {
    spinlock: &'a Spinlock<T>,
}

impl<'a, T: Send> SpinlockGuard<'a, T> {
    fn drop(&self) {
        self.spinlock.flag.store(false, Ordering::Release);
    }
}

impl<'a, T: Send> Deref for SpinlockGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.spinlock.cell.get() }
    }
}

impl<'a, T: Send> DerefMut for SpinlockGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.spinlock.cell.get() }
    }
}

impl<T: Send> Spinlock<T> {
    pub fn new(value: T) -> Spinlock<T> {
        Spinlock{
            cell: UnsafeCell::new(value),
            flag: AtomicBool::new(false),
        }
    }

    pub fn lock(&self) -> SpinlockGuard<T> {
        while !self.flag.compare_and_swap(false, true, Ordering::Acquire) {}
        SpinlockGuard{ spinlock: self }
    }
}




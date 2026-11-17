use std::ops::{Deref, DerefMut};

use parking_lot::{Mutex, MutexGuard};

#[derive(Default)]
pub struct RunLock<T>(Mutex<T>);

pub struct RunGuard<'a, T>(MutexGuard<'a, T>);

impl<T> RunLock<T> {
    pub fn new(value: T) -> Self {
        Self(Mutex::new(value))
    }

    pub fn lock(&self) -> RunGuard<'_, T> {
        RunGuard(self.0.lock())
    }
}

impl<T> Deref for RunGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> DerefMut for RunGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

//! 后台绘制结果合并唤醒；没有定时器，也不为每个结果累积主线程消息。
use iced::{
    futures::{stream, task::AtomicWaker, Stream},
    Subscription,
};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    task::Poll,
};

/// 有界后台结果通道；结果实际入队后立即合并唤醒绘制，避免等下一轮相机定时帧。
pub(crate) struct Sender<T>(mpsc::SyncSender<T>);
impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<T> Sender<T> {
    pub(crate) fn send(&self, value: T) -> Result<(), mpsc::SendError<T>> {
        self.0.send(value)?;
        notify();
        Ok(())
    }
}
pub(crate) fn bounded<T>(capacity: usize) -> (Sender<T>, mpsc::Receiver<T>) {
    let (sender, receiver) = mpsc::sync_channel(capacity);
    (Sender(sender), receiver)
}

static READY: AtomicBool = AtomicBool::new(false);
static WAKE: AtomicWaker = AtomicWaker::new();

pub(crate) fn notify() {
    READY.store(true, Ordering::Release);
    WAKE.wake();
}
pub(crate) fn events() -> impl Stream<Item = ()> {
    stream::poll_fn(|cx| {
        WAKE.register(cx.waker());
        if READY.swap(false, Ordering::AcqRel) {
            Poll::Ready(Some(()))
        } else {
            Poll::Pending
        }
    })
}
pub(crate) fn subscription() -> Subscription<()> {
    Subscription::run(events)
}

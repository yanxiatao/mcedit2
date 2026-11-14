//! 区域来源分组在发布前异步冻结；取消旧目标会释放队列中和在途的定义工作。
use super::{Definition, Key, StreamMesh};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{channel, Receiver, TryRecvError},
    Arc, Mutex,
};

type Definitions = Vec<(Key, Arc<Definition>)>;
pub(in crate::gpu_scene) struct Plan {
    cancelled: Arc<AtomicBool>,
    dispatch: crate::render_work::Task,
    receiver: Mutex<Receiver<Option<Definitions>>>,
}
impl Plan {
    pub fn start(stream: Arc<StreamMesh>) -> Result<Self, String> {
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&cancelled);
        let (sender, receiver) = channel();
        let dispatch = crate::render_work::spawn_source(
            "opaque-regions",
            Arc::clone(&cancelled),
            move || {
                let result = super::definitions(&stream, &|| flag.load(Ordering::Acquire));
                if sender.send(result).is_err() && !flag.load(Ordering::Acquire) {
                    log::debug!("[gpu-upload] 区域定义接收端已关闭");
                }
            },
        )?;
        Ok(Self {
            cancelled,
            dispatch,
            receiver: Mutex::new(receiver),
        })
    }
    pub fn poll(&mut self) -> Result<Option<Definitions>, String> {
        if !self.dispatch.submit()? {
            return Ok(None);
        }
        match self
            .receiver
            .get_mut()
            .expect("区域定义接收锁异常")
            .try_recv()
        {
            Ok(Some(definitions)) => Ok(Some(definitions)),
            Ok(None) => Err("区域来源定义已取消".into()),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err("区域定义工作线程未返回结果".into()),
        }
    }
}
impl Drop for Plan {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

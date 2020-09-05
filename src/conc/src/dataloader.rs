use async_trait::async_trait;
use std::boxed::Box;
use std::clone::Clone;
use std::cmp::{Ord, Ordering};
use std::collections::{BTreeSet, HashMap};
use std::future::Future;
use std::marker::Send;
use std::mem;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::{Duration};
use std::vec::Vec;
use tokio::select;
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tokio::time::{delay_for, Delay};

pub type ID = i32;

type AccessFrame = HashMap<ID, u32>;

const MAX_FRAMES: usize = 20;

pub struct CacheItem<T> {
    item: T,
    accessed: u32,
}

#[derive(Eq)]
pub struct AccessInfo {
    id: ID,
    accessed: u32,
}

impl Ord for AccessInfo {
    fn cmp(&self, other: &Self) -> Ordering {
        if self.id < other.id {
            Ordering::Less
        } else if self.id > other.id {
            Ordering::Greater
        } else {
            Ordering::Equal
        }
    }
}

impl PartialOrd for AccessInfo {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for AccessInfo {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

pub struct Cache<T> {
    items: HashMap<ID, T>,
    total: BTreeSet<AccessInfo>,
    frames: Vec<AccessFrame>,
    current_frame: usize,
    max_size: usize,
    max_frames: usize,
}

impl<T> Cache<T> {
    fn new(max_size: usize, max_frames: usize) -> Cache<T> {
        return Cache {
            items: HashMap::new(),
            total: BTreeSet::new(),
            frames: vec![HashMap::new()],
            current_frame: 0,
            max_size: max_size,
            max_frames: max_frames,
        };
    }

    fn modify_total(total: &mut BTreeSet<AccessInfo>, id: ID, incr: i32) {
        let key = &AccessInfo {
            id: id,
            accessed: 0,
        };

        if let Some(info) = total.take(key) {
            total.insert(AccessInfo {
                id: info.id,
                accessed: (info.accessed as i32 + incr) as u32,
            });
        }
    }

    fn insert(&mut self, id: ID, item: T, accessed: i32) {
        if self.total.len() >= self.max_size {
            self.items.remove(&id);
            self.total.remove(&AccessInfo {
                id: id,
                accessed: 0,
            });
        }

        self.items.insert(id, item);
        Self::modify_total(&mut self.total, id, accessed);
    }

    fn get(&mut self, id: ID, item: T, accessed: i32) -> Option<&T> {
        Self::modify_total(&mut self.total, id, accessed);
        self.items.get(&id)
    }

    fn access(&mut self, id: ID, count: u32) {
        Self::modify_total(&mut self.total, id, count as i32);
        let curr_frame = &mut self.frames[self.current_frame];
        match curr_frame.get_mut(&id) {
            Some(accessed) => *accessed += count,
            None => {
                curr_frame.insert(id, count);
            }
        };
    }

    fn next_cache_frame(&mut self) {
        /*
        for (id, count) in &mut self.frames[self.current_frame] {
            Self::modify_total(&mut self.total, *id, *count as i32);
        }*/

        if self.frames.len() < self.max_frames {
            self.frames.push(HashMap::new());
        }

        self.current_frame = (self.current_frame + 1) % MAX_FRAMES;

        //Clear current frame
        for (id, count) in &self.frames[self.current_frame] {
            Self::modify_total(&mut self.total, *id, -(*count as i32));
        }

        self.frames[self.current_frame].clear();
    }
}

#[derive(Clone)]
pub enum DataResult<T> {
    Ok(T),
    Internal(String),
    Error(String),
    Pending,
}

impl<T> DataResult<T> {
    fn take(&mut self) -> DataResult<T> {
        mem::replace(self, DataResult::Pending)
    }
}

struct DataLoaderFutureShared<T: Clone + Send> {
    result: DataResult<T>,
    waker: Option<Waker>,
}

pub struct DataLoaderFuture<T: Clone + Send> {
    id: ID,
    shared: Arc<Mutex<DataLoaderFutureShared<T>>>,
}

impl<T: Clone + Send> Unpin for DataLoaderFuture<T> {}

//this is way to allocation heavy!
impl<T: Clone + Send> Future for DataLoaderFuture<T> {
    type Output = Result<T, String>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let shared = &mut self.shared.lock().unwrap();

        match shared.result.take() {
            DataResult::Ok(v) => Poll::Ready(Ok(v)),
            DataResult::Error(e) => Poll::Ready(Err(e)),
            DataResult::Internal(e) => {
                eprintln!("INTERNAL ERROR {}", e);
                Poll::Ready(Err("Internal Error".to_string()))
            }

            DataResult::Pending => {
                shared.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }
}

#[derive(Clone)]
pub struct DataLoaderEndpoint<T: Clone + Send>(Sender<DataLoaderFuture<T>>);

impl<T: Clone + Send> DataLoaderEndpoint<T> {
    pub async fn load(&mut self, id: ID) -> Result<T, String> {
        let shared = Arc::new(Mutex::new(DataLoaderFutureShared {
            result: DataResult::Pending,
            waker: None,
        }));

        let send_t = self.0
            .send(DataLoaderFuture {
                id: id,
                shared: shared.clone(),
            });

        if let Err(_) = send_t.await {
            return Err("Data loader channel is closed".to_string())
        }

        DataLoaderFuture {
            id: id,
            shared: shared.clone(),
        }
        .await
    }
}

/*
pub type DataLoaderFunc<
    F: Fn(&SharedContext, &mut Hashmap<ID, DataResult<T>>) -> Fut,
    T: Clone + Send,
    Fut: Future<Output = ()>,
> = F;*/
//fn(ctx: &SharedContext, results: &mut HashMap<ID, DataResult<T>>) -> F;

#[async_trait]
pub trait DataLoaderHandler<T, C> {
    async fn batch_execute(
        &mut self,
        ctx: &C,
        results: &mut HashMap<ID, DataResult<T>>,
    );
}

pub struct DataLoader<
    F: DataLoaderHandler<T, C>, //Fn(&SharedContext, &mut HashMap<ID, DataResult<T>>) -> Fut,
    C: Send + Sync,
    T: Clone + Send,
    //Fut: Future<Output = ()>,
> {
    loader: F,
    results: HashMap<ID, DataResult<T>>,
    futures: Vec<DataLoaderFuture<T>>,
    rx: Receiver<DataLoaderFuture<T>>,
    batch_size: usize,
    timeout: Duration,
    timeout_t: Delay,
    phantom: std::marker::PhantomData<C>,
}

impl<
        F: 'static + Send + DataLoaderHandler<T,C>, // Fn(&SharedContext, &mut HashMap<ID, DataResult<T>>) -> Fut,
        C: 'static + Send + Sync,
        T: 'static + Clone + Send,
        //Fut: Future<Output = ()>,
    > DataLoader<F, C, T>
{
    fn infinite_t() -> Delay {
        //delay_for(Duration::from_millis(1))
        delay_for(Duration::from_secs(100000))
    }

    pub fn new(
        func: F,
        shared_ctx: Arc<C>,
        batch_size: usize,
        timeout: Duration,
    ) -> DataLoaderEndpoint<T> {
        let (tx, rx) = channel(batch_size);

        tokio::spawn(
            DataLoader {
                //cache: Cache::new(100, 10),
                results: HashMap::new(),
                futures: Vec::with_capacity(batch_size),
                loader: func,
                rx: rx,
                batch_size: batch_size,
                timeout: timeout,
                timeout_t: Self::infinite_t(),
                phantom: std::marker::PhantomData,
            }
            .run(shared_ctx),
        );

        DataLoaderEndpoint(tx)
    }

    pub async fn run(mut self, ctx: Arc<C>) {
        let mut timeout = delay_for(self.timeout);

        loop {
            select! {
                recv = self.rx.recv() => {
                    let future = match recv {
                        Some(future) => future,
                        None => return
                    };

                    let id = future.id;
                    self.futures.push(future);

                    if self.results.get(&id).is_some() { continue }

                    self.results.insert(id, DataResult::Pending);

                    let len = self.results.len();
                    if len == 1 {
                        timeout = delay_for(self.timeout);
                        println!("Delay for {:?}", self.timeout);
                    }

                    if len == self.batch_size {
                        self.batch_execute(&ctx).await;
                    }
                }
                _ = &mut timeout, if self.futures.len() > 0 => {
                    self.batch_execute(&ctx).await;
                }
            }
        }
    }

    async fn batch_execute(&mut self, ctx: &C) {
        self.loader.batch_execute(ctx, &mut self.results).await;

        //with a retain mut could merge into one loop
        for future in &mut self.futures {
            let shared = &mut future.shared.lock().unwrap();
            shared.result = self.results[&future.id].clone();

            if let Some(waker) = shared.waker.take() {
                waker.wake()
            }
        }

        self.results.clear();
        self.futures.clear();
        self.timeout_t = Self::infinite_t();

        /*
        self.futures.retain(|future| {
            let shared = future.shared.lock().unwrap();
            match &shared.result {
                DataResult::Pending => {
                    println!("Result still pending!");
                    true
                },
                _ => false,
            }
        });*/

        println!("Executed batch");
    }
}

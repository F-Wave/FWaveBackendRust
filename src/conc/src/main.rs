use std::sync::atomic::Ordering;

pub mod mpmc;
pub mod spinlock;

const BUFFER_SIZE: usize = 16;
const BUFFER_MUL: usize = 5;
const NUM_PRODUCERS: usize = 10;

//+nightly bench -- --nocapture

fn main() {
    let (sender, mut receiver) = mpmc::channel(BUFFER_SIZE as u32);

    let mut runtime = tokio::runtime::Runtime::new().expect("Unable to create a runtime");

    for i in 0..200000 {
        println!("========================");
        runtime.block_on(async {
            let mut joins = Vec::with_capacity(NUM_PRODUCERS);

            for i in 0..NUM_PRODUCERS {
                let mut sender = sender.clone();
                let mut receiver = receiver.clone();

                tokio::spawn(async move {
                    for i in 0..BUFFER_SIZE * BUFFER_MUL {
                        sender.send(i).await;
                    }
                });

                joins.push(tokio::spawn(async move {
                    for i in 0..BUFFER_SIZE * BUFFER_MUL {
                        let val = receiver.recv().await;
                    }
                }));
            }

            for join in joins {
                tokio::join!(join);
            }
        });
    };
}

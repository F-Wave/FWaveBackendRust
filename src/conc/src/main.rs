pub mod mpmc;
pub mod spinlock;

const BUFFER_SIZE: usize = 10;
const BUFFER_MUL: usize = 5;
const NUM_PRODUCERS: usize = 10;

//+nightly bench -- --nocapture

fn main() {
    let (sender, mut receiver) = mpmc::channel(BUFFER_SIZE);

    let mut runtime = tokio::runtime::Runtime::new().expect("Unable to create a runtime");

    for i in 0..10000 {
        runtime.block_on(
            async {
                let mut joins = Vec::with_capacity(NUM_PRODUCERS);

                for p in 0..NUM_PRODUCERS {
                    let mut sender = sender.clone();

                    tokio::spawn(async move {
                        for i in 0..BUFFER_SIZE * BUFFER_MUL {
                            sender.send(format!("producer {}, sent {}", p, i)).await;
                        }
                    });

                    let mut receiver = receiver.clone();

                    joins.push(tokio::spawn(async move {
                        for i in 0..BUFFER_SIZE * BUFFER_MUL {
                            //println!("Getting read number {}", i);
                            let val = receiver.recv().await;

                            //println!("Got val {}", val);
                        }
                    }));
                }

                for join in joins {
                    tokio::join!(join);
                }
            });
    };
}

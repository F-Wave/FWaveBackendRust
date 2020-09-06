#[cfg(test)]
mod tests {
    extern crate test;

    use test::Bencher;
    use tokio::sync::mpsc;
    use crate::mpmc;
    use crate::mpmc::*;

    const BUFFER_SIZE: usize = 1000;
    const BUFFER_MUL: usize = 10;
    const NUM_PRODUCERS: usize = 8;

    #[bench]
    fn bench_mpsc(b: &mut Bencher) {
        let (sender, mut receiver) = mpsc::channel(BUFFER_SIZE);

        let mut runtime = tokio::runtime::Runtime::new().expect("Unable to create a runtime");

        b.iter(move || {
            runtime.block_on(
            async {
                for i in 0..NUM_PRODUCERS {
                    let mut sender = sender.clone();

                    tokio::spawn(async move {
                        for i in 0..BUFFER_SIZE * BUFFER_MUL {
                            sender.send(i).await;
                        }
                    });
                }

                for i in 0..BUFFER_SIZE * BUFFER_MUL * NUM_PRODUCERS {
                    let val = receiver.recv().await;
                }
            });
        });
    }


    #[bench]
    fn bench_mpmc(b: &mut Bencher) {
        let (sender, mut receiver) = mpmc::channel(BUFFER_SIZE);

        let mut runtime = tokio::runtime::Runtime::new().expect("Unable to create a runtime");

        b.iter(move || {
            runtime.block_on(
                async {
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

                    /*for i in 0..BUFFER_SIZE * BUFFER_MUL * NUM_PRODUCERS {
                        let val = receiver.recv().await;
                    }*/

                    for join in joins {
                        tokio::join!(join);
                    }
                });
        });
    }

    #[test]
    fn test_mpmc() {
       tokio_test::block_on(async {
           let (mut send, mut recv) = mpmc::channel(2);


           let join = tokio::spawn(async move {
               for i in 0..10 {
                   assert_eq!(recv.recv().await, i);
               }
           });

           tokio::spawn(async move {
               for i in 0..10 {
                   send.send(i).await;
               }
           });

           tokio::join!(join);
       })
    }
}
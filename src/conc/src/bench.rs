#[cfg(test)]
mod tests {
    extern crate test;

    use test::Bencher;
    use tokio::sync::mpsc;
    use crate::mpmc;
    use crate::mpmc::*;

    const buffer_size: usize = 100;
    const buffer_mul: usize = 10;
    const num_producers: usize = 50;



    #[bench]
    fn bench_mpsc(b: &mut Bencher) {
        let (sender, mut receiver) = mpsc::channel(buffer_size);

        b.iter(move || {
            tokio_test::block_on(
            async {
                for i in 0..num_producers {
                    let mut sender = sender.clone();

                    tokio::spawn(async move {
                        for i in 0..buffer_size * buffer_mul {
                            sender.send(i).await;
                        }
                    });
                }

                for i in 0..buffer_size * buffer_mul * num_producers {
                    let val = receiver.recv().await;
                }
            });
        });
    }


    #[bench]
    fn bench_mpmc(b: &mut Bencher) {
        let (sender, mut receiver) = mpmc::channel(buffer_size);

        b.iter(move || {
            tokio_test::block_on(
                async {
                    for i in 0..num_producers {
                        let mut sender = sender.clone();

                        tokio::spawn(async move {
                            for i in 0..buffer_size * buffer_mul {
                                sender.send(i).await;
                            }
                        });
                    }

                    for i in 0..buffer_size * buffer_mul * num_producers {
                        let val = receiver.recv().await;
                    }
                });
        });
    }

    #[test]
    fn test_mpmc() {
       tokio_test::block_on(async {
           let (mut send, mut recv) = mpmc::channel(10);

           tokio::spawn(async move {
               for i in 0..3 {
                   send.send(i).await;
               }
           });

           for i in 0..3 {
               assert_eq!(recv.recv().await, i);
           }
       })
    }
}
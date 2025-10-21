//! Benchmark suite for Semaphore performance
//!
//! Measures baseline performance for different contention scenarios.

use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use compio_sync::Semaphore;
use std::sync::Arc;

fn bench_uncontended_try_acquire(c: &mut Criterion) {
    c.bench_function("semaphore/uncontended/try_acquire", |b| {
        let sem = Semaphore::new(100);
        b.iter(|| {
            let p = sem.try_acquire();
            black_box(p);
        });
    });
}

fn bench_uncontended_acquire(c: &mut Criterion) {
    c.bench_function("semaphore/uncontended/acquire", |b| {
        b.iter(|| {
            compio::runtime::Runtime::new().unwrap().block_on(async {
                let sem = Semaphore::new(100);
                let p = sem.acquire().await;
                black_box(p);
            });
        });
    });
}

fn bench_contended_varying_concurrency(c: &mut Criterion) {
    let mut group = c.benchmark_group("semaphore/contended");
    
    for concurrency in [2, 4, 8, 16, 32, 64].iter() {
        group.bench_with_input(
            BenchmarkId::from_parameter(concurrency),
            concurrency,
            |b, &concurrency| {
                b.iter(|| {
                    compio::runtime::Runtime::new().unwrap().block_on(async {
                        let sem = Arc::new(Semaphore::new(4));
                        let mut handles = vec![];
                        
                        for _ in 0..concurrency {
                            let sem = sem.clone();
                            handles.push(compio::runtime::spawn(async move {
                                let _p = sem.acquire().await;
                                black_box(42);
                            }));
                        }
                        
                        for h in handles {
                            h.await.unwrap();
                        }
                    });
                });
            },
        );
    }
    
    group.finish();
}

fn bench_acquire_release_cycles(c: &mut Criterion) {
    c.bench_function("semaphore/cycles/1000_iterations", |b| {
        b.iter(|| {
            compio::runtime::Runtime::new().unwrap().block_on(async {
                let sem = Semaphore::new(1);
                for _ in 0..1000 {
                    let p = sem.acquire().await;
                    drop(p);
                }
            });
        });
    });
}

fn bench_high_permits_low_contention(c: &mut Criterion) {
    c.bench_function("semaphore/high_permits/acquire_100_of_1000", |b| {
        b.iter(|| {
            compio::runtime::Runtime::new().unwrap().block_on(async {
                let sem = Arc::new(Semaphore::new(1000));
                let mut handles = vec![];
                
                // Only 100 concurrent acquires on 1000 permits
                for _ in 0..100 {
                    let sem = sem.clone();
                    handles.push(compio::runtime::spawn(async move {
                        let _p = sem.acquire().await;
                        black_box(42);
                    }));
                }
                
                for h in handles {
                    h.await.unwrap();
                }
            });
        });
    });
}

criterion_group!(
    benches,
    bench_uncontended_try_acquire,
    bench_uncontended_acquire,
    bench_contended_varying_concurrency,
    bench_acquire_release_cycles,
    bench_high_permits_low_contention
);
criterion_main!(benches);


use std::hint::black_box;

use serc_benchmark::WORKLOADS;
use serc_benchmark::criterion::{
    BatchSize, Criterion, Throughput, criterion_group, criterion_main,
};

fn compiler(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("compile");

    for workload in WORKLOADS {
        serc::compile(workload.source, workload.filename).unwrap();

        group.throughput(Throughput::Bytes(workload.source.len() as u64));
        group.bench_function(workload.id, |bencher| {
            bencher.iter_batched(
                || (),
                |()| serc::compile(black_box(workload.source), black_box(workload.filename)),
                BatchSize::LargeInput,
            );
        });
    }

    group.finish();

    let mut group = criterion.benchmark_group("marshal");

    for workload in WORKLOADS {
        let module = serc::compile(workload.source, workload.filename).unwrap();

        group.throughput(Throughput::Bytes(workload.source.len() as u64));
        group.bench_function(workload.id, |bencher| {
            bencher.iter_batched(
                || (),
                |()| black_box(&module).marshal(),
                BatchSize::LargeInput,
            );
        });
    }

    group.finish();
}

criterion_group!(benches, compiler);
criterion_main!(benches);

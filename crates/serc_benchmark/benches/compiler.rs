use std::hint::black_box;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use serc_benchmark::criterion::{
    BatchSize, Criterion, Throughput, criterion_group, criterion_main,
};
use serc_benchmark::{
    COMPILE_MICRO_WORKLOADS, COMPOSITE_WORKLOADS, LARGE_WORKLOADS, MARSHAL_MICRO_WORKLOADS,
};

fn compiler(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("compile");

    for workload in COMPILE_MICRO_WORKLOADS
        .iter()
        .chain(COMPOSITE_WORKLOADS)
        .chain(LARGE_WORKLOADS)
    {
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

    for workload in MARSHAL_MICRO_WORKLOADS
        .iter()
        .chain(COMPOSITE_WORKLOADS)
        .chain(LARGE_WORKLOADS)
    {
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

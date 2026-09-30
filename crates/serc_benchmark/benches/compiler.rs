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
}

fn marshal_once(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("marshal_once");

    for workload in MARSHAL_MICRO_WORKLOADS
        .iter()
        .chain(COMPOSITE_WORKLOADS)
        .chain(LARGE_WORKLOADS)
    {
        group.throughput(Throughput::Bytes(workload.source.len() as u64));
        group.bench_function(workload.id, |bencher| {
            bencher.iter_batched(
                // A freshly compiled module has no serialization work left over from warmup
                // or earlier iterations. Cloning could preserve a future shared cache.
                || serc::compile(workload.source, workload.filename).unwrap(),
                |module| {
                    let output = black_box(&module).marshal();
                    // Keep cleanup outside timing when isolating the serialization phase.
                    (module, output)
                },
                BatchSize::LargeInput,
            );
        });
    }

    group.finish();
}

fn compile_pyc(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("compile_pyc");
    for workload in COMPILE_MICRO_WORKLOADS
        .iter()
        .chain(MARSHAL_MICRO_WORKLOADS)
        .chain(COMPOSITE_WORKLOADS)
        .chain(LARGE_WORKLOADS)
    {
        let source_size = u32::try_from(workload.source.len())
            .expect("benchmark source must fit in a PYC source-size field");
        group.throughput(Throughput::Bytes(workload.source.len() as u64));
        group.bench_function(workload.id, |bencher| {
            bencher.iter(|| {
                let module =
                    serc::compile(black_box(workload.source), black_box(workload.filename))
                        .unwrap();
                // Include encoding and both drops in this one-shot workflow. The PYC stays
                // in memory so disk I/O does not hide compiler or serializer changes.
                black_box(module.to_timestamp_pyc(0, source_size));
            });
        });
    }
    group.finish();
}

criterion_group!(benches, compiler, marshal_once, compile_pyc);
criterion_main!(benches);

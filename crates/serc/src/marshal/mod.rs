mod graph;
mod key;
mod plan;
use serc_bytecode::marshal::{v4, v5, v6};
mod writer;

#[cfg(all(test, feature = "decompile"))]
mod reader_tests;
#[cfg(test)]
mod tests;

pub(crate) use graph::{code_graph_summary, code_occurrence_bound, validate_initial_intern_state};
pub(crate) use key::{
    ConstantHashes, bytes_hash, constant_hashes_for_code, constants_tuple_hash,
    metadata_tuple_properties,
};
#[cfg(test)]
pub(crate) use writer::encode_code;
pub(crate) use writer::{encode_code_with_options, encode_pyc_code_with_options};

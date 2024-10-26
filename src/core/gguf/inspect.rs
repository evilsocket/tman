use std::{collections::HashSet, path::PathBuf};

use rayon::prelude::*;

use crate::{
    cli::DetailLevel,
    core::{FileType, Inspection, Metadata, TensorDescriptor},
};

use super::data_type_bits;

pub(crate) fn inspect(
    file_path: PathBuf,
    detail: DetailLevel,
    filter: Option<String>,
) -> anyhow::Result<Inspection> {
    let mut inspection = Inspection::default();

    let file = std::fs::File::open(&file_path)?;
    let buffer = unsafe {
        memmap2::MmapOptions::new()
            .map(&file)
            .unwrap_or_else(|_| panic!("failed to map file {}", file_path.display()))
    };

    inspection.file_path = file_path.canonicalize()?;
    inspection.file_size = file.metadata()?.len();

    let gguf = gguf::GGUFFile::read(&buffer)
        .map_err(|e| anyhow::anyhow!("failed to read GGUF file: {}", e))?
        .unwrap_or_else(|| panic!("failed to read GGUF file {}", file_path.display()));

    inspection.file_type = FileType::GGUF;
    inspection.version = format!("{}", gguf.header.version);
    inspection.num_tensors = gguf.header.tensor_count as usize;
    inspection.unique_shapes = gguf
        .tensors
        .par_iter()
        .map(|t| t.dimensions.iter().map(|d| *d as usize).collect::<Vec<_>>())
        .filter(|shape| !shape.is_empty())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    // sort shapes by volume
    inspection.unique_shapes.sort_by(|a, b| {
        let size_a: usize = a.iter().product();
        let size_b: usize = b.iter().product();
        size_a.cmp(&size_b)
    });

    inspection.unique_dtypes = gguf
        .tensors
        .par_iter()
        .map(|t| format!("{:?}", t.tensor_type))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    inspection.data_size = gguf
        .tensors
        .par_iter()
        .map(|t| {
            if t.dimensions.is_empty() {
                0
            } else {
                data_type_bits(t.tensor_type)
                    * t.dimensions.iter().map(|d| *d as usize).product::<usize>()
            }
        })
        .sum::<usize>()
        / 8;

    for meta in &gguf.header.metadata {
        inspection
            .metadata
            .insert(meta.key.clone(), format!("{:?}", meta.value));
    }

    if matches!(detail, DetailLevel::Full) {
        let mut tensor_descriptors = Vec::new();

        for t_info in &gguf.tensors {
            if let Some(filter) = &filter {
                if !t_info.name.contains(filter) {
                    continue;
                }
            }

            tensor_descriptors.push(TensorDescriptor {
                id: Some(t_info.name.to_string()),
                shape: t_info.dimensions.iter().map(|d| *d as usize).collect(),
                dtype: format!("{:?}", t_info.tensor_type),
                size: if t_info.dimensions.is_empty() {
                    0
                } else {
                    (data_type_bits(t_info.tensor_type)
                        * t_info
                            .dimensions
                            .iter()
                            .map(|d| *d as usize)
                            .product::<usize>())
                        / 8
                },
                metadata: Metadata::new(),
            });
        }

        inspection.tensors = Some(tensor_descriptors);
    }

    Ok(inspection)
}

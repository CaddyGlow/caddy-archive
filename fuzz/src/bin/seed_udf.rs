//! Generate complete, bounded UDF media and replayable writer recipes.
use archive_fuzz::udf::{IMAGE_LIMIT, read_once, repair_tags, seed_image, seed_recipes};
use libmkiso::{AllocationMode, UdfImage, UdfOptions, UdfPartition, UdfRevision};
use std::{error::Error, fs, path::Path};

fn main() -> Result<(), Box<dyn Error>> {
    let output = std::env::args()
        .nth(1)
        .ok_or("usage: seed_udf OUTPUT_ROOT")?;
    let output = Path::new(&output);
    fs::create_dir_all(output.join("udf"))?;
    fs::create_dir_all(output.join("udf_roundtrip"))?;
    let recipes = seed_recipes();
    for (name, recipe) in &recipes {
        let image = seed_image(recipe).ok_or_else(|| format!("valid seed rejected: {name}"))?;
        assert!(image.len() <= IMAGE_LIMIT);
        fs::write(output.join("udf").join(format!("{name}.udf")), image)?;
        fs::write(output.join("udf_roundtrip").join(name), recipe)?;
    }
    // Enough metadata blocks to span fragments, while staying below reader entry budgets.
    let directory = tempfile::tempdir()?;
    let image_path = directory.path().join("metadata.udf");
    let mut tree = UdfImage::new();
    for index in 0..80 {
        tree.add_bytes(format!("file-{index:03}"), vec![index as u8; 31])?;
    }
    tree.write(
        &image_path,
        &UdfOptions {
            revision: UdfRevision::V260,
            partition: UdfPartition::Metadata { mirror: true },
            allocation: AllocationMode::Long,
            metadata_extent_blocks: 32,
            max_image_bytes: IMAGE_LIMIT as u64,
            ..Default::default()
        },
    )?;
    let bytes = fs::read(image_path)?;
    assert!(read_once(&bytes).opened);
    fs::write(output.join("udf/metadata-fragments.udf"), &bytes)?;
    let mut damaged = bytes.clone();
    damaged[(320 + 32) * 2048 + 112] ^= 1;
    assert!(read_once(&damaged).opened);
    fs::write(output.join("udf/metadata-mirror-recovery.udf"), damaged)?;
    let mut chained = bytes;
    let fe = 320 * 2048;
    let length = u32::from_le_bytes(chained[fe + 172..fe + 176].try_into()?) as usize;
    let allocations = chained[fe + 176..fe + 176 + length].to_vec();
    let aed_block = 64u32; // Reserved gap following the first metadata fragment.
    let aed = (320 + aed_block as usize) * 2048;
    chained[aed..aed + 2048].fill(0);
    chained[aed..aed + 2].copy_from_slice(&258u16.to_le_bytes());
    chained[aed + 2..aed + 4].copy_from_slice(&3u16.to_le_bytes());
    chained[aed + 6..aed + 8].copy_from_slice(&1u16.to_le_bytes());
    chained[aed + 10..aed + 12].copy_from_slice(&((8 + length) as u16).to_le_bytes());
    chained[aed + 12..aed + 16].copy_from_slice(&aed_block.to_le_bytes());
    chained[aed + 20..aed + 24].copy_from_slice(&(length as u32).to_le_bytes());
    chained[aed + 24..aed + 24 + length].copy_from_slice(&allocations);
    chained[fe + 176..fe + 2048].fill(0);
    chained[fe + 172..fe + 176].copy_from_slice(&8u32.to_le_bytes());
    chained[fe + 176..fe + 180].copy_from_slice(&0xc0000800u32.to_le_bytes());
    chained[fe + 180..fe + 184].copy_from_slice(&aed_block.to_le_bytes());
    chained[fe + 10..fe + 12].copy_from_slice(&168u16.to_le_bytes());
    repair_tags(&mut chained);
    assert!(read_once(&chained).opened);
    fs::write(output.join("udf/metadata-allocation-chain.udf"), chained)?;
    println!(
        "Generated {} complete UDF images and {} writer recipes",
        recipes.len() + 3,
        recipes.len()
    );
    Ok(())
}

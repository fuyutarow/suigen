pub mod framework_sources;
pub mod gen;
pub mod manifest;
pub mod model_builder;
pub mod package_cache;
pub mod ts_imports;

use anyhow::Result;
use manifest::GenManifest;
use std::path::Path;

pub type TokenWriter = fn(s: &str, path: &Path) -> Result<()>;

pub async fn gen_ts(
    manifest: &GenManifest,
    out_root: &Path,
    token_writer: TokenWriter,
) -> Result<()> {
    let model_args = model_builder::BuildModelArgs {
        manifest,
        out_root,
        token_writer,
    };

    model_builder::build_model(model_args).await
}

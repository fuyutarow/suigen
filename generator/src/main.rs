use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::*;
use colored::*;
use convert_case::{Case, Casing};
use genco::fmt;
use genco::prelude::*;
use move_core_types::account_address::AccountAddress;
use move_model_2::{compiled_model, model, source_model};
use move_package::source_package::parsed_manifest::PackageName;
use move_symbol_pool::Symbol;
use std::io::Write;
use sui_move_build::SuiPackageHooks;
use sui_sdk::SuiClientBuilder;
use suigen::framework_sources;
use suigen::gen::{
    gen_init_loader_ts, gen_package_init_ts, module_import_name, package_import_name,
};
use suigen::gen::{FrameworkImportCtx, FunctionsGen, StructClassImportCtx, StructsGen};
use suigen::manifest::{parse_gen_manifest_from_file, GenManifest, Package};
use suigen::model_builder::{
    build_models, OnChainModelResult, SourceModelResult, TypeOriginTable, VersionTable,
};
use suigen::package_cache::PackageCache;

const DEFAULT_RPC: &str = "https://fullnode.mainnet.sui.io:443";

#[derive(Parser)]
#[clap(
    name = "suigen",
    version,
    about = "Generate TS SDKs for Sui Move smart contracts."
)]
struct Args {
    #[arg(
        short,
        long,
        help = "Path to the `gen.toml` file.",
        default_value = "./gen.toml"
    )]
    manifest: String,

    #[arg(
        short,
        long,
        help = "Path to the output directory. If omitted, the current directory will be used.",
        default_value = "."
    )]
    out: String,

    #[arg(
        long,
        help = "Remove all contents of the output directory before generating, except for gen.toml. Use with caution."
    )]
    clean: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    move_package::package_hooks::register_package_hooks(Box::new(SuiPackageHooks));

    let manifest = parse_gen_manifest_from_file(Path::new(&args.manifest))?;
    let rpc_url = match &manifest.config {
        Some(config) => config
            .rpc
            .clone()
            .unwrap_or_else(|| DEFAULT_RPC.to_string()),
        None => DEFAULT_RPC.to_string(),
    };
    let rpc_client = SuiClientBuilder::default().build(rpc_url).await?;

    let mut progress_output = std::io::stderr();

    // build models
    let mut cache = PackageCache::new(rpc_client.read_api());
    let (source_model, on_chain_model) = build_models(
        &mut cache,
        &manifest.packages,
        &PathBuf::from(&args.manifest),
        &mut progress_output,
    )
    .await?;

    if source_model.is_none() && on_chain_model.is_none() {
        writeln!(std::io::stderr(), "No packages to generate.")?;
        return Ok(());
    }

    // clean output
    if args.clean {
        clean_output(&PathBuf::from(&args.out))?;
    }

    // separate modules by package
    let source_pkgs: BTreeMap<AccountAddress, source_model::Package> = source_model
        .as_ref()
        .map(|m| m.env.packages().map(|pkg| (pkg.address(), pkg)).collect())
        .unwrap_or_default();

    let on_chain_pkgs: BTreeMap<AccountAddress, compiled_model::Package> = on_chain_model
        .as_ref()
        .map(|m| m.env.packages().map(|pkg| (pkg.address(), pkg)).collect())
        .unwrap_or_default();

    // gen top-level packages and dependencies
    let (source_top_level_addr_map, on_chain_top_level_addr_map) =
        resolve_top_level_pkg_addr_map(&source_model, &on_chain_model, &manifest);

    // gen _framework
    writeln!(progress_output, "{}", "GENERATING FRAMEWORK".green().bold())?;

    let out_root = PathBuf::from(args.out);
    std::fs::create_dir_all(&out_root)?;

    std::fs::create_dir_all(out_root.join("_framework"))?;
    write_str_to_file(
        framework_sources::LOADER,
        out_root.join("_framework").join("loader.ts").as_ref(),
    )?;
    write_str_to_file(
        framework_sources::UTIL,
        out_root.join("_framework").join("util.ts").as_ref(),
    )?;
    write_str_to_file(
        framework_sources::REIFIED,
        out_root.join("_framework").join("reified.ts").as_ref(),
    )?;
    write_str_to_file(
        framework_sources::VECTOR,
        out_root.join("_framework").join("vector.ts").as_ref(),
    )?;
    write_tokens_to_file(
        &gen_init_loader_ts(
            match source_pkgs.is_empty() {
                false => Some((
                    source_pkgs.keys().copied().collect::<Vec<_>>(),
                    &source_top_level_addr_map,
                )),
                true => None,
            },
            match on_chain_pkgs.is_empty() {
                false => Some((
                    on_chain_pkgs.keys().copied().collect::<Vec<_>>(),
                    &on_chain_top_level_addr_map,
                )),
                true => None,
            },
        ),
        out_root.join("_framework").join("init-loader.ts").as_ref(),
    )?;

    if let Some(m) = &source_model {
        writeln!(
            progress_output,
            "{}",
            "GENERATING SOURCE PACKAGES".green().bold()
        )?;
        gen_packages_for_model(
            source_pkgs,
            &source_top_level_addr_map,
            &m.published_at,
            &m.type_origin_table,
            &m.version_table,
            true,
            &out_root,
        )?;
    }
    if let Some(m) = &on_chain_model {
        writeln!(
            progress_output,
            "{}",
            "GENERATING ON-CHAIN PACKAGES".green().bold()
        )?;
        gen_packages_for_model(
            on_chain_pkgs,
            &on_chain_top_level_addr_map,
            &m.published_at,
            &m.type_origin_table,
            &m.version_table,
            false,
            &out_root,
        )?;
    }

    // gen .eslintrc.json
    write_str_to_file(
        framework_sources::ESLINTRC,
        &out_root.join(".eslintrc.json"),
    )?;

    // Generate a top-level barrel file that re-exports all packages
    gen_top_level_barrel_file(
        &out_root,
        &source_top_level_addr_map,
        &on_chain_top_level_addr_map,
    )?;

    Ok(())
}

/// Generates a top-level barrel file (index.ts) that re-exports all packages
fn gen_top_level_barrel_file(
    out_root: &Path,
    source_top_level_pkg_names: &BTreeMap<AccountAddress, Symbol>,
    on_chain_top_level_pkg_names: &BTreeMap<AccountAddress, Symbol>,
) -> Result<()> {
    let mut barrel_content = String::new();
    let mut exported_names = BTreeSet::new();

    // Helper function to generate a safe import name that handles JavaScript reserved words
    let get_safe_import_name = |pkg_name: Symbol| {
        // Use package_import_name for package-level exports
        let import_name = package_import_name(pkg_name);
        // Check if the import name is a JavaScript reserved word
        if suigen::gen::JS_RESERVED_WORDS.contains(&import_name.as_str()) {
            format!("{}_pkg", import_name)
        } else {
            import_name
        }
    };

    // Helper function to generate PascalCase path for imports
    let get_pascal_case_path = |pkg_name: Symbol| {
        let name = pkg_name.to_string();
        // If the name contains underscores, it's likely in snake_case format
        if name.contains('_') {
            name.from_case(Case::Snake).to_case(Case::Pascal)
        } else {
            // Already in PascalCase or ensure it is
            name.from_case(Case::Camel).to_case(Case::Pascal)
        }
    };

    // Add exports for source packages
    for (_, pkg_name) in source_top_level_pkg_names.iter() {
        let safe_import_name = get_safe_import_name(*pkg_name);
        let pascal_case_path = get_pascal_case_path(*pkg_name);

        // Skip if we've already exported this name (avoids duplicates)
        if !exported_names.insert(safe_import_name.clone()) {
            continue;
        }

        barrel_content.push_str(&format!(
            "export * as {} from './{}';\n",
            safe_import_name, pascal_case_path
        ));
    }

    // Add exports for on-chain packages
    for (_, pkg_name) in on_chain_top_level_pkg_names.iter() {
        let safe_import_name = get_safe_import_name(*pkg_name);
        let pascal_case_path = get_pascal_case_path(*pkg_name);

        // Skip if we've already exported this name (avoids duplicates)
        if !exported_names.insert(safe_import_name.clone()) {
            continue;
        }

        barrel_content.push_str(&format!(
            "export * as {} from './{}';\n",
            safe_import_name, pascal_case_path
        ));
    }

    // Write the barrel file with package exports header
    if !barrel_content.is_empty() {
        // Add a comment indicating these are package exports
        let barrel_content_with_header = format!("// Package exports\n{}", barrel_content);
        write_str_to_file(&barrel_content_with_header, &out_root.join("index.ts"))?;
    }

    Ok(())
}

fn clean_output(out_root: &Path) -> Result<()> {
    let mut paths_to_remove = vec![];
    for entry in std::fs::read_dir(out_root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() && path.file_name().unwrap() == "gen.toml" {
            continue;
        }
        paths_to_remove.push(path);
    }

    for path in paths_to_remove {
        if path.is_dir() {
            std::fs::remove_dir_all(path)?;
        } else {
            std::fs::remove_file(path)?;
        }
    }

    Ok(())
}

fn write_tokens_to_file(tokens: &Tokens<JavaScript>, path: &Path) -> Result<()> {
    if tokens.is_empty() {
        return Ok(());
    }

    let file = std::fs::File::create(path)?;
    let mut w = fmt::IoWriter::new(file);
    let fmt = fmt::Config::from_lang::<JavaScript>();
    let config = js::Config::default();
    tokens.format_file(&mut w.as_formatter(&fmt), &config)?;
    Ok(())
}

fn write_str_to_file(s: &str, path: &Path) -> Result<()> {
    if s.is_empty() {
        return Ok(());
    }

    let file = std::fs::File::create(path)?;
    let mut w = fmt::IoWriter::new(file);
    std::fmt::Write::write_str(&mut w, s)?;
    Ok(())
}

/// Creates a mapping between address and package name for top-level packages.
fn resolve_top_level_pkg_addr_map(
    source_model: &Option<SourceModelResult>,
    on_chain_model: &Option<OnChainModelResult>,
    manifest: &GenManifest,
) -> (
    BTreeMap<AccountAddress, Symbol>,
    BTreeMap<AccountAddress, Symbol>,
) {
    let mut source_top_level_package_names: BTreeSet<PackageName> = BTreeSet::new();
    let mut on_chain_top_level_package_names: BTreeSet<PackageName> = BTreeSet::new();
    for (name, pkg) in manifest.packages.iter() {
        match pkg {
            Package::Dependency(_) => {
                source_top_level_package_names.insert(*name);
            }
            Package::OnChain(_) => {
                on_chain_top_level_package_names.insert(*name);
            }
        }
    }

    let source_top_level_id_map: BTreeMap<AccountAddress, Symbol> = if let Some(m) = source_model {
        m.id_map
            .iter()
            .filter_map(|(id, name)| {
                if source_top_level_package_names.contains(name) {
                    Some((*id, *name))
                } else {
                    None
                }
            })
            .collect()
    } else {
        BTreeMap::new()
    };

    let on_chain_top_level_id_map: BTreeMap<AccountAddress, Symbol> =
        if let Some(m) = on_chain_model {
            m.id_map
                .iter()
                .filter_map(|(id, name)| {
                    if on_chain_top_level_package_names.contains(name) {
                        Some((*id, *name))
                    } else {
                        None
                    }
                })
                .collect()
        } else {
            BTreeMap::new()
        };

    (source_top_level_id_map, on_chain_top_level_id_map)
}

fn gen_packages_for_model<const HAS_SOURCE: usize>(
    pkgs: BTreeMap<AccountAddress, model::Package<HAS_SOURCE>>,
    top_level_pkg_names: &BTreeMap<AccountAddress, Symbol>,
    published_at_map: &BTreeMap<AccountAddress, AccountAddress>,
    type_origin_table: &TypeOriginTable,
    version_table: &VersionTable,
    is_source: bool,
    out_root: &Path,
) -> Result<()> {
    if pkgs.is_empty() {
        return Ok(());
    }

    for (pkg_id, pkg) in pkgs.iter() {
        let is_top_level = top_level_pkg_names.contains_key(pkg_id);
        let levels_from_root = if is_top_level { 0 } else { 2 };

        // Get the safe package import name
        let safe_pkg_import_name = match top_level_pkg_names.get(pkg_id) {
            Some(pkg_name) => {
                let import_name = package_import_name(*pkg_name);
                // Check if the import name is a JavaScript reserved word
                if suigen::gen::JS_RESERVED_WORDS.contains(&import_name.as_str()) {
                    format!("{}_pkg", import_name)
                } else {
                    import_name
                }
            }
            None => {
                let dep_dir = match is_source {
                    true => "source",
                    false => "onchain",
                };
                format!("{}/{}", dep_dir, pkg_id.to_hex_literal())
            }
        };

        // Convert package name to PascalCase for directory name
        let pascal_case_pkg_name = match top_level_pkg_names.get(pkg_id) {
            Some(pkg_name) => {
                let name = pkg_name.to_string();
                // If the name contains underscores, it's likely in snake_case format
                if name.contains('_') {
                    name.from_case(Case::Snake).to_case(Case::Pascal)
                } else {
                    // Already in PascalCase or ensure it is
                    name.from_case(Case::Camel).to_case(Case::Pascal)
                }
            }
            None => {
                let dep_dir = match is_source {
                    true => "source",
                    false => "onchain",
                };
                format!("{}/{}", dep_dir, pkg_id.to_hex_literal())
            }
        };

        let package_path = out_root.join(match top_level_pkg_names.get(pkg_id) {
            Some(_) => PathBuf::from(pascal_case_pkg_name),
            None => PathBuf::from("_dependencies")
                .join(match is_source {
                    true => "source",
                    false => "onchain",
                })
                .join(pkg_id.to_hex_literal()),
        });

        std::fs::create_dir_all(&package_path)?;

        // Generate module paths for the export statements and check if each module has files
        let mut validated_modules = Vec::new();
        for module in pkg.modules() {
            let module_name = module_import_name(module.name());
            let original_module_name = module.name().to_string();
            let module_path = package_path.join(&original_module_name);

            // Create the module directory
            std::fs::create_dir_all(&module_path)?;

            // Only include modules that will contain files
            // Check if the module has any functions or structs
            let has_functions = module.functions().next().is_some();
            let has_structs = module.structs().next().is_some();

            if has_functions || has_structs {
                validated_modules.push((module, module_name, original_module_name));
            }
        }

        // Generate constants.ts with package metadata
        let published_at = published_at_map.get(pkg_id).unwrap_or(pkg_id);
        let versions = version_table.get(pkg_id).unwrap();
        let mut constants_content = format!(
            "export const PACKAGE_ID = '{}';\n\
             export const PUBLISHED_AT = '{}';\n",
            pkg_id.to_hex_literal(),
            published_at.to_hex_literal()
        );

        // Add version exports to constants.ts
        for (published_at, version) in versions {
            constants_content.push_str(&format!(
                "export const PKG_V{} = '{}';\n",
                version.value(),
                published_at.to_hex_literal()
            ));
        }

        write_str_to_file(&constants_content, &package_path.join("constants.ts"))?;

        // generate index.ts that re-exports constants and modules
        let mut index_content =
            String::from("// Re-export package constants\nexport * from './constants';\n\n");

        // Add module exports with namespaces, but only for modules with files
        if !validated_modules.is_empty() {
            index_content.push_str("// Module exports\n");
            for (_, module_name, original_module_name) in &validated_modules {
                // Convert module name to camelCase for export name
                let camel_case_module_name = module_name.clone();

                // Check if module name is a reserved word and add suffix if needed
                let safe_module_name =
                    if suigen::gen::JS_RESERVED_WORDS.contains(&camel_case_module_name.as_str()) {
                        format!("{}_mod", camel_case_module_name)
                    } else {
                        camel_case_module_name
                    };

                index_content.push_str(&format!(
                    "export * as {} from './{}';\n",
                    safe_module_name, original_module_name
                ));
            }
        }

        write_str_to_file(&index_content, &package_path.join("index.ts"))?;

        // generate init.ts
        let tokens = gen_package_init_ts(pkg, &FrameworkImportCtx::new(levels_from_root + 1));
        write_tokens_to_file(&tokens, &package_path.join("init.ts"))?;

        // generate modules
        for (module, module_name, original_module_name) in validated_modules {
            let module_path = package_path.join(&original_module_name);

            // generate <module>/functions.ts
            if is_top_level {
                let mut tokens = js::Tokens::new();
                let mut import_ctx =
                    &mut StructClassImportCtx::for_func_gen(&module, top_level_pkg_names);
                for func in module.functions() {
                    let func_gen_res = FunctionsGen::new(
                        import_ctx,
                        FrameworkImportCtx::new(levels_from_root + 2),
                        func,
                    );
                    let mut func_gen = match func_gen_res {
                        Ok(func_gen) => func_gen,
                        Err(ic) => {
                            import_ctx = ic;
                            continue;
                        }
                    };
                    func_gen.gen_fun_args_if(&mut tokens)?;
                    func_gen.gen_fun_binding(&mut tokens)?;
                    import_ctx = func_gen.import_ctx;
                }
                write_tokens_to_file(&tokens, &module_path.join("functions.ts"))?;
            }

            // generate <module>/structs.ts
            let mut tokens = js::Tokens::new();
            let mut import_ctx =
                &mut StructClassImportCtx::for_struct_gen(&module, top_level_pkg_names);

            for strct in module.structs() {
                let mut structs_gen = StructsGen::new(
                    import_ctx,
                    FrameworkImportCtx::new(levels_from_root + 2),
                    type_origin_table,
                    version_table,
                    strct,
                );
                structs_gen.gen_struct_sep_comment(&mut tokens);

                // type check function
                structs_gen.gen_is_type_func(&mut tokens);

                // fields interface
                structs_gen.gen_fields_if(&mut tokens);

                // struct class
                structs_gen.gen_struct_class(&mut tokens);
                import_ctx = structs_gen.import_ctx;
            }
            write_tokens_to_file(&tokens, &module_path.join("structs.ts"))?;

            // generate <module>/index.ts (barrel file)
            gen_module_barrel_file(&module_path)?;
        }
    }

    Ok(())
}

/// Generates a barrel file (index.ts) for a module that re-exports all exports from the module's files
fn gen_module_barrel_file(module_path: &Path) -> Result<()> {
    let module_dir = std::fs::read_dir(module_path)?;
    let mut export_files = Vec::new();

    // Find all TypeScript files in the module directory (excluding any existing index.ts)
    for entry in module_dir {
        let entry = entry?;
        let path = entry.path();

        if path.is_file()
            && path.extension().is_some_and(|ext| ext == "ts")
            && path.file_name().is_some_and(|name| name != "index.ts")
        {
            if let Some(file_stem) = path.file_stem() {
                if let Some(file_stem_str) = file_stem.to_str() {
                    export_files.push(file_stem_str.to_string());
                }
            }
        }
    }

    // Skip creating barrel file if there are no files to export
    if export_files.is_empty() {
        return Ok(());
    }

    // Generate the barrel file content with re-exports
    let mut barrel_content = String::new();
    for file in export_files {
        // Use direct exports for 'structs' and 'functions', namespace exports for others
        if file == "structs" || file == "functions" {
            barrel_content.push_str(&format!("export * from './{}';\n", file));
        } else {
            // Convert the export name to camelCase but keep the import path as is
            let camel_case_name = module_import_name(Symbol::from(file.as_str()));
            barrel_content.push_str(&format!(
                "export * as {} from './{}';\n",
                camel_case_name, file
            ));
        }
    }

    // Write the barrel file
    write_str_to_file(&barrel_content, &module_path.join("index.ts"))?;

    Ok(())
}

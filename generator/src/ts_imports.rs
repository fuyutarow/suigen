use genco::prelude::*;
use std::collections::HashMap;
use std::path::Path;

/// Creates a TypeScript import with an option to specify a type-only import
pub struct TypedImport {
    pub module: String,
    pub name: String,
    pub alias: Option<String>,
    pub is_type_only: bool,
}

/// Creates a TypeScript import statement with an option to specify a type-only import
pub fn ts_import(module: impl ToString, name: impl ToString, type_only: bool) -> TypedImport {
    TypedImport {
        module: module.to_string(),
        name: name.to_string(),
        alias: None,
        is_type_only: type_only,
    }
}

/// Generate TypeScript import statement for type declarations
pub fn generate_type_declarations_imports() -> String {
    r#"
// Type-only imports
import type {
    PhantomReified,
    PhantomToTypeStr,
    PhantomTypeArgument,
    Reified,
    StructClass,
    ToField,
    ToPhantomTypeArgument,
    ToTypeStr
} from "../../_framework/reified";

import type { FieldsWithTypes } from "../../_framework/util";
import type { SuiClient, SuiObjectData, SuiParsedData } from "@mysten/sui/client";
import type { TransactionArgument, TransactionObjectInput } from "@mysten/sui/transactions";
"#
    .to_string()
}

/// Manually format import into a JavaScript token
pub fn format_typed_import(imports: &[TypedImport]) -> js::Tokens {
    let mut tokens = js::Tokens::new();

    // Group imports by module and type
    let mut imports_by_module: HashMap<String, (Vec<TypedImport>, Vec<TypedImport>)> =
        HashMap::new();

    for import in imports {
        let entry = imports_by_module
            .entry(import.module.clone())
            .or_insert_with(|| (Vec::new(), Vec::new()));

        if import.is_type_only {
            entry.1.push(import.clone());
        } else {
            entry.0.push(import.clone());
        }
    }

    // Format each group of imports
    for (module, (regular_imports, type_imports)) in imports_by_module {
        // Format regular imports
        if !regular_imports.is_empty() {
            let import_names = regular_imports
                .iter()
                .map(|imp| match &imp.alias {
                    Some(alias) => format!("{} as {}", imp.name, alias),
                    None => imp.name.clone(),
                })
                .collect::<Vec<_>>()
                .join(", ");

            tokens.append(format!(
                "import {{ {} }} from \"{}\";",
                import_names, module
            ));
            tokens.push();
        }

        // Format type imports
        if !type_imports.is_empty() {
            let import_names = type_imports
                .iter()
                .map(|imp| match &imp.alias {
                    Some(alias) => format!("{} as {}", imp.name, alias),
                    None => imp.name.clone(),
                })
                .collect::<Vec<_>>()
                .join(", ");

            tokens.append(format!(
                "import type {{ {} }} from \"{}\";",
                import_names, module
            ));
            tokens.push();
        }
    }

    tokens
}

impl Clone for TypedImport {
    fn clone(&self) -> Self {
        TypedImport {
            module: self.module.clone(),
            name: self.name.clone(),
            alias: self.alias.clone(),
            is_type_only: self.is_type_only,
        }
    }
}

/// Helper function to write a collection of TypedImports to a file
pub fn write_typescript_imports(file_path: &Path, imports: &[TypedImport]) -> std::io::Result<()> {
    // Convert imports to a string
    let mut import_content = String::new();

    // Group imports by module and type
    let mut imports_by_module: HashMap<String, (Vec<&TypedImport>, Vec<&TypedImport>)> =
        HashMap::new();

    for import in imports {
        let entry = imports_by_module
            .entry(import.module.clone())
            .or_insert_with(|| (Vec::new(), Vec::new()));

        if import.is_type_only {
            entry.1.push(import);
        } else {
            entry.0.push(import);
        }
    }

    // Format each group of imports
    for (module, (regular_imports, type_imports)) in imports_by_module {
        // Format regular imports
        if !regular_imports.is_empty() {
            let import_names = regular_imports
                .iter()
                .map(|imp| match &imp.alias {
                    Some(alias) => format!("{} as {}", imp.name, alias),
                    None => imp.name.clone(),
                })
                .collect::<Vec<_>>()
                .join(", ");

            import_content.push_str(&format!(
                "import {{ {} }} from \"{}\";\n",
                import_names, module
            ));
        }

        // Format type imports
        if !type_imports.is_empty() {
            let import_names = type_imports
                .iter()
                .map(|imp| match &imp.alias {
                    Some(alias) => format!("{} as {}", imp.name, alias),
                    None => imp.name.clone(),
                })
                .collect::<Vec<_>>()
                .join(", ");

            import_content.push_str(&format!(
                "import type {{ {} }} from \"{}\";\n",
                import_names, module
            ));
        }
    }

    std::fs::write(file_path, import_content)
}

use std::{borrow::Cow, fmt};

use serde::Serialize;
use silex_builder::Builder;
use silex_types::Type;
use xelis_common::contract::{ContractVersion, build_environment};

use crate::JsonStorage;

/// Owned environment metadata, printable as Silex declarations and serializable as JSON.
/// Type strings use Silex syntax. Each collection is sorted by owner type and name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EnvironmentListing {
    pub schema_version: u32,
    pub contract_version: ContractVersion,
    pub functions: Vec<FunctionDeclaration>,
    pub methods: Vec<FunctionDeclaration>,
    pub static_functions: Vec<FunctionDeclaration>,
    pub constants: Vec<ConstantDeclaration>,
    pub constant_functions: Vec<ConstantFunctionDeclaration>,
    pub structures: Vec<StructureDeclaration>,
    pub enums: Vec<EnumDeclaration>,
    pub opaque_types: Vec<OpaqueDeclaration>,
    pub hooks: Vec<HookDeclaration>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NamedType {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FunctionDeclaration {
    pub id: u16,
    pub name: String,
    pub on_type: Option<String>,
    pub parameters: Vec<NamedType>,
    /// None means the function has no return value.
    pub return_type: Option<String>,
    pub require_instance: bool,
    pub gas_cost: u64,
    pub documentation: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ConstantDeclaration {
    pub name: String,
    pub on_type: String,
    #[serde(rename = "type")]
    pub ty: String,
    /// Display representation: preserves large integers; opaque values show their type.
    pub value: String,
    pub documentation: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ConstantFunctionDeclaration {
    pub name: String,
    pub on_type: String,
    pub parameters: Vec<NamedType>,
    // Return types are unavailable in the environment metadata.
    pub documentation: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StructureDeclaration {
    pub id: u16,
    pub name: String,
    pub generics: Vec<String>,
    pub fields: Vec<NamedType>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EnumDeclaration {
    pub id: u16,
    pub name: String,
    pub generics: Vec<String>,
    pub variants: Vec<VariantDeclaration>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VariantDeclaration {
    pub id: u8,
    pub name: String,
    pub is_tuple: bool,
    pub fields: Vec<NamedType>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OpaqueDeclaration {
    pub id: u16,
    pub name: String,
    pub generics_count: u8,
    pub allow_external_input: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HookDeclaration {
    pub id: u8,
    pub name: String,
    pub parameters: Vec<NamedType>,
    pub return_type: Option<String>,
}

/// Inspect the builder before runtime construction discards declaration names.
/// The returned metadata owns its strings and does not retain the environment.
///
/// ```
/// use silex_cli::environment::listing;
/// use xelis_common::contract::ContractVersion;
///
/// let environment = listing(ContractVersion::V1);
/// println!("{environment}");
/// let json = serde_json::json!(&environment);
/// assert_eq!(json["contract_version"], "v1");
/// assert!(environment.functions.iter().any(|function| function.name == "println"));
/// ```
pub fn listing(version: ContractVersion) -> EnvironmentListing {
    let environment = build_environment::<JsonStorage>(version);
    let mut listing = EnvironmentListing {
        schema_version: 1,
        contract_version: version,
        functions: Vec::new(),
        methods: Vec::new(),
        static_functions: Vec::new(),
        constants: Vec::new(),
        constant_functions: Vec::new(),
        structures: Vec::new(),
        enums: Vec::new(),
        opaque_types: Vec::new(),
        hooks: Vec::new(),
    };
    for declarations in environment
        .get_functions_mapper()
        .get_declared_functions()
        .values()
    {
        for (function, id) in declarations {
            let declaration = FunctionDeclaration {
                id: *id,
                name: function.name.to_owned(),
                on_type: function.on_type.as_ref().map(ToString::to_string),
                parameters: parameters(&function.parameters),
                return_type: function.return_type.as_ref().map(ToString::to_string),
                require_instance: function.require_instance,
                gas_cost: function.cost,
                documentation: function.comment.map(str::to_owned),
            };
            if function.on_type.is_none() {
                listing.functions.push(declaration);
            } else if function.require_instance {
                listing.methods.push(declaration);
            } else {
                listing.static_functions.push(declaration);
            }
        }
    }
    for (ty, declarations) in environment.get_constants() {
        for (name, constant) in declarations {
            listing.constants.push(ConstantDeclaration {
                name: (*name).to_owned(),
                on_type: ty.to_string(),
                ty: constant.value_type().to_string(),
                value: constant.value().to_string(),
                documentation: constant.comment().map(str::to_owned),
            });
        }
    }
    for (ty, declarations) in environment.get_const_functions_mapper().get_mappings() {
        for function in declarations.values() {
            listing
                .constant_functions
                .push(ConstantFunctionDeclaration {
                    name: function.name.to_owned(),
                    on_type: ty.to_string(),
                    parameters: parameters(&function.parameters),
                    documentation: function.comment.map(str::to_owned),
                });
        }
    }
    listing.structures = environment
        .get_structs()
        .map(|builder| {
            let ty = builder.get_type();
            StructureDeclaration {
                id: ty.id(),
                name: ty.name().to_owned(),
                generics: ty.generics().iter().map(ToString::to_string).collect(),
                fields: fields(ty.fields()),
            }
        })
        .collect();
    listing.enums = environment
        .get_enums()
        .map(|builder| {
            let ty = builder.get_type();
            EnumDeclaration {
                id: ty.id(),
                name: ty.name().to_owned(),
                generics: ty.generics().iter().map(ToString::to_string).collect(),
                variants: ty
                    .variants()
                    .iter()
                    .enumerate()
                    .map(|(id, (name, variant))| VariantDeclaration {
                        id: id as u8,
                        name: name.to_string(),
                        is_tuple: variant.is_tuple(),
                        fields: fields(variant.fields()),
                    })
                    .collect(),
            }
        })
        .collect();
    listing.opaque_types = environment
        .get_opaque_manager()
        .iter()
        .map(|ty| OpaqueDeclaration {
            id: ty.id(),
            name: ty.name().to_owned(),
            generics_count: ty.generics_count(),
            allow_external_input: ty.allow_external_input(),
        })
        .collect();
    listing.hooks = environment
        .get_hooks()
        .iter()
        .map(|(name, hook)| HookDeclaration {
            id: hook.hook_id,
            name: (*name).to_owned(),
            parameters: parameters(&hook.parameters),
            return_type: hook.return_type.as_ref().map(ToString::to_string),
        })
        .collect();
    for functions in [
        &mut listing.functions,
        &mut listing.methods,
        &mut listing.static_functions,
    ] {
        functions.sort_by(|a, b| (&a.on_type, &a.name).cmp(&(&b.on_type, &b.name)));
    }
    listing
        .constants
        .sort_by(|a, b| (&a.on_type, &a.name).cmp(&(&b.on_type, &b.name)));
    listing
        .constant_functions
        .sort_by(|a, b| (&a.on_type, &a.name).cmp(&(&b.on_type, &b.name)));
    listing.structures.sort_by(|a, b| a.name.cmp(&b.name));
    listing.enums.sort_by(|a, b| a.name.cmp(&b.name));
    listing.opaque_types.sort_by(|a, b| a.name.cmp(&b.name));
    listing.hooks.sort_by(|a, b| a.name.cmp(&b.name));
    listing
}

fn parameters(parameters: &[(&str, Type)]) -> Vec<NamedType> {
    parameters
        .iter()
        .map(|(name, ty)| NamedType {
            name: (*name).to_owned(),
            ty: ty.to_string(),
        })
        .collect()
}

fn fields(fields: &[(Cow<'static, str>, Type)]) -> Vec<NamedType> {
    fields
        .iter()
        .map(|(name, ty)| NamedType {
            name: name.to_string(),
            ty: ty.to_string(),
        })
        .collect()
}

impl fmt::Display for NamedType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.name, self.ty)
    }
}

fn signature(
    f: &mut fmt::Formatter<'_>,
    name: &str,
    parameters: &[NamedType],
    return_type: Option<&str>,
) -> fmt::Result {
    write!(f, "{name}(")?;
    separated(f, parameters.iter())?;
    write!(f, ")")?;
    if let Some(ty) = return_type {
        write!(f, " -> {ty}")?;
    }
    Ok(())
}

fn separated(
    f: &mut fmt::Formatter<'_>,
    items: impl Iterator<Item = impl fmt::Display>,
) -> fmt::Result {
    for (index, item) in items.enumerate() {
        if index > 0 {
            write!(f, ", ")?;
        }
        write!(f, "{item}")?;
    }
    Ok(())
}

impl fmt::Display for FunctionDeclaration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(ty) = &self.on_type {
            write!(f, "{ty}::")?;
        }
        signature(f, &self.name, &self.parameters, self.return_type.as_deref())?;
        write!(f, " [gas: {}]", self.gas_cost)
    }
}

impl fmt::Display for ConstantDeclaration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}::{}: {} = {}",
            self.on_type, self.name, self.ty, self.value
        )
    }
}

impl fmt::Display for ConstantFunctionDeclaration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}::", self.on_type)?;
        signature(f, &self.name, &self.parameters, None)
    }
}

fn type_name(f: &mut fmt::Formatter<'_>, name: &str, generics: &[String]) -> fmt::Result {
    write!(f, "{name}")?;
    if !generics.is_empty() {
        write!(f, "<")?;
        separated(f, generics.iter())?;
        write!(f, ">")?;
    }
    Ok(())
}

impl fmt::Display for StructureDeclaration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "struct ")?;
        type_name(f, &self.name, &self.generics)?;
        write!(f, " {{ ")?;
        separated(f, self.fields.iter())?;
        write!(f, " }}")
    }
}

impl fmt::Display for VariantDeclaration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name)?;
        if self.is_tuple {
            write!(f, "(")?;
            separated(f, self.fields.iter().map(|field| &field.ty))?;
            write!(f, ")")?;
        } else if !self.fields.is_empty() {
            write!(f, " {{ ")?;
            separated(f, self.fields.iter())?;
            write!(f, " }}")?;
        }
        Ok(())
    }
}

impl fmt::Display for EnumDeclaration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "enum ")?;
        type_name(f, &self.name, &self.generics)?;
        write!(f, " {{ ")?;
        separated(f, self.variants.iter())?;
        write!(f, " }}")
    }
}

impl fmt::Display for OpaqueDeclaration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name)
    }
}

impl fmt::Display for HookDeclaration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        signature(f, &self.name, &self.parameters, self.return_type.as_deref())
    }
}

fn section(
    f: &mut fmt::Formatter<'_>,
    title: &str,
    declarations: &[impl fmt::Display],
) -> fmt::Result {
    write!(f, "\n{title}:\n")?;
    if declarations.is_empty() {
        writeln!(f, "  (none)")?;
    }
    // Preserve the original text ordering, which sorts complete declarations.
    let mut lines: Vec<_> = declarations.iter().map(ToString::to_string).collect();
    lines.sort();
    for line in lines {
        writeln!(f, "  {line}")?;
    }
    Ok(())
}

impl fmt::Display for EnvironmentListing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Contract version: {}", self.contract_version)?;
        section(f, "Functions", &self.functions)?;
        section(f, "Methods (instance required)", &self.methods)?;
        section(f, "Static functions", &self.static_functions)?;
        section(f, "Constants", &self.constants)?;
        section(f, "Constant functions", &self.constant_functions)?;
        section(f, "Structures", &self.structures)?;
        section(f, "Enums", &self.enums)?;
        section(f, "Opaque types", &self.opaque_types)?;
        section(f, "Hooks", &self.hooks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    #[test]
    fn json_metadata_supports_editor_validation() {
        let metadata = json!(listing(ContractVersion::V1));
        let encoded = serde_json::to_string(&metadata).unwrap();
        assert_eq!(serde_json::from_str::<Value>(&encoded).unwrap(), metadata);
        assert_eq!(metadata, json!(listing(ContractVersion::V1)));
        assert_eq!(metadata["schema_version"], 1);
        assert_eq!(metadata["contract_version"], "v1");
        let transfer = metadata["functions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|function| function["name"] == "transfer_payload")
            .unwrap();
        assert_eq!(
            transfer["parameters"][1],
            json!({"name": "amount", "type": "u64"})
        );
        assert_eq!(transfer["return_type"], "bool");
        assert_eq!(transfer["require_instance"], false);
        assert!(transfer["documentation"].is_string());
        assert!(
            !json!(listing(ContractVersion::V0))["functions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|function| function["name"] == "transfer_payload")
        );
        let max = metadata["constants"]
            .as_array()
            .unwrap()
            .iter()
            .find(|constant| constant["name"] == "MAX" && constant["on_type"] == "u128")
            .unwrap();
        assert_eq!(max["value"], u128::MAX.to_string());
        let entry = metadata["structures"]
            .as_array()
            .unwrap()
            .iter()
            .find(|ty| ty["name"] == "Entry")
            .unwrap();
        assert_eq!(entry["fields"][0], json!({"name": "key", "type": "bytes"}));
        assert!(
            metadata["methods"]
                .as_array()
                .unwrap()
                .iter()
                .all(|method| method["require_instance"] == true)
        );
    }

    #[test]
    fn listing_respects_version_and_is_deterministic() {
        let v0 = listing(ContractVersion::V0).to_string();
        let v1 = listing(ContractVersion::V1).to_string();
        assert!(!v0.contains("transfer_payload("));
        assert!(v1.contains("transfer_payload(destination:"));
        assert!(v1.contains("println("));
        assert!(v1.contains("constructor() -> u64"));
        assert_eq!(v1, listing(ContractVersion::V1).to_string());
        for section in [
            "Functions",
            "Methods (instance required)",
            "Static functions",
            "Constants",
            "Constant functions",
            "Structures",
            "Enums",
            "Opaque types",
            "Hooks",
        ] {
            assert!(v1.contains(&format!("\n{section}:\n")));
        }
    }
}

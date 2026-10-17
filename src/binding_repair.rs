//! Declarative binding inspection and scoped, draft-only repair. Never changes
//! visual behavior, producer trust, connection configuration, or saved intent.
use crate::connection_registry::ConnectionRegistry;
use crate::setup::{Activation, SetupDocument};
use crate::source_capabilities::CapabilityWarning;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BindingKey {
    pub layer_id: String,
    /// None addresses the layer default; Some addresses one stable variant ID.
    pub variant_id: Option<String>,
}

pub struct Binding<'a> {
    pub key: BindingKey,
    pub layer_name: &'a str,
    pub variant_name: Option<&'a str>,
    pub enabled: bool,
    pub source: &'a str,
    pub activation: &'a Activation,
}

pub fn activation_source(activation: &Activation) -> Option<&str> {
    match activation {
        Activation::Always {} => None,
        Activation::WhileState { source, .. }
        | Activation::OnEvent { source, .. }
        | Activation::OnTransition { source, .. } => Some(source),
    }
}

/// Includes disabled layers AND disabled variants, in bottom-to-top order.
pub fn bindings(setup: &SetupDocument) -> Vec<Binding<'_>> {
    setup
        .layers
        .iter()
        .flat_map(|layer| {
            std::iter::once((&layer.activation, None, None, layer.enabled))
                .chain(layer.variants.iter().map(|v| {
                    (
                        &v.activation,
                        Some(v.id.clone()),
                        Some(v.name.as_str()),
                        layer.enabled && v.enabled,
                    )
                }))
                .filter_map(|(activation, variant_id, variant_name, enabled)| {
                    Some(Binding {
                        key: BindingKey {
                            layer_id: layer.id.clone(),
                            variant_id,
                        },
                        layer_name: &layer.name,
                        variant_name,
                        enabled,
                        source: activation_source(activation)?,
                        activation,
                    })
                })
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct BindingRepair {
    pub setup_id: String,
    pub key: BindingKey,
    /// Detect stale dialogs without discarding unrelated subsequent draft edits.
    pub expected_activation: Activation,
    pub source: String,
    /// Explicit choice: None = any instance, Some = exact producer identity.
    pub instance: Option<String>,
}

/// No fact remapping: undeclared events/states survive with warnings. Never
/// interprets similarly named facts across sources as equivalent.
pub fn repair_binding(
    setup: &SetupDocument,
    registry: &ConnectionRegistry,
    repair: &BindingRepair,
) -> Result<(SetupDocument, Vec<CapabilityWarning>), String> {
    if setup.id != repair.setup_id {
        return Err("composition changed; reopen binding repair".into());
    }
    registry.validate()?;
    if registry.connection(&repair.source).is_none() {
        return Err("choose a configured connection; missing bindings remain intact".into());
    }
    let mut candidate = setup.clone();
    let layer = candidate
        .layers
        .iter_mut()
        .find(|l| l.id == repair.key.layer_id)
        .ok_or("layer no longer exists; reopen binding repair")?;
    let activation = if let Some(id) = &repair.key.variant_id {
        &mut layer
            .variants
            .iter_mut()
            .find(|v| &v.id == id)
            .ok_or("variant no longer exists; reopen binding repair")?
            .activation
    } else {
        &mut layer.activation
    };
    if *activation != repair.expected_activation {
        return Err("binding changed since repair was opened; reopen and review".into());
    }
    match activation {
        Activation::Always {} => return Err("always-active layers have no source binding".into()),
        Activation::WhileState {
            source, instance, ..
        }
        | Activation::OnEvent {
            source, instance, ..
        }
        | Activation::OnTransition {
            source, instance, ..
        } => {
            *source = repair.source.clone();
            *instance = repair.instance.clone();
        }
    }
    let warnings = registry
        .capability_registry()?
        .validate_activation(activation)?;
    candidate.validate()?;
    Ok((candidate, warnings))
}

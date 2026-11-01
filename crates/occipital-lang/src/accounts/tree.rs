use super::{Accounts, Refused, RetainedAllocation, Retention, Unit, UnitKind};
use crate::eval::tree::{MarkdownKind, Tree, TreeNode, TreeStep, TreeValue};
use crate::DisplayText;
use std::sync::Arc;

impl Accounts {
    /// Promote a completed evaluation's temporary tree to retained storage.
    /// Price backing capacities, not logical language sizes, before publication.
    pub fn retain_tree(&self, tree: &Tree) -> Result<Arc<Retention>, Refused> {
        let mut unit = self
            .begin(UnitKind::Evaluation)
            .map_err(Refused::Aggregate)?;
        nodes(&mut unit, &tree.blocks)?;
        // An empty tree still owns a frame header and an ownership ledger.
        unit.retain(RetainedAllocation {
            requested: (std::mem::size_of::<Tree>() + 16) as u64,
            unused_capacity: 0,
        })?;
        unit.finish()
    }
}

fn buffer<T>(unit: &mut Unit, values: &Vec<T>) -> Result<(), Refused> {
    if values.capacity() == 0 {
        return Ok(());
    }
    let slot = std::mem::size_of::<T>() as u64;
    unit.retain(RetainedAllocation {
        requested: (values.len() as u64)
            .checked_mul(slot)
            .ok_or(Refused::UnitFuel)?,
        unused_capacity: ((values.capacity() - values.len()) as u64)
            .checked_mul(slot)
            .ok_or(Refused::UnitFuel)?,
    })
}

fn text(unit: &mut Unit, text: &DisplayText) -> Result<(), Refused> {
    if let Some(allocation) = text.retained_allocation() {
        unit.retain(allocation)?;
    }
    Ok(())
}

fn nodes(unit: &mut Unit, values: &Vec<TreeNode>) -> Result<(), Refused> {
    buffer(unit, values)?;
    for value in values {
        node(unit, value)?;
    }
    Ok(())
}

fn node(unit: &mut Unit, value: &TreeNode) -> Result<(), Refused> {
    match value {
        TreeNode::Markdown { kind, children } => {
            match kind {
                MarkdownKind::CodeBlock {
                    language,
                    text: body,
                } => {
                    text(unit, language)?;
                    text(unit, body)?;
                }
                MarkdownKind::Code(body) => text(unit, body)?,
                MarkdownKind::Link {
                    destination, title, ..
                } => {
                    text(unit, destination)?;
                    text(unit, title)?;
                }
                _ => {}
            }
            nodes(unit, children)?;
        }
        TreeNode::Component {
            name,
            props,
            children,
        } => {
            text(unit, name)?;
            record(unit, props)?;
            nodes(unit, children)?;
        }
        TreeNode::Text(body) | TreeNode::Reference(body) => text(unit, body)?,
        TreeNode::ErrorCell { name, .. } => text(unit, name)?,
        TreeNode::ErrorMarker(_) => {}
    }
    Ok(())
}

fn record(unit: &mut Unit, fields: &Vec<(DisplayText, TreeValue)>) -> Result<(), Refused> {
    buffer(unit, fields)?;
    for (name, value) in fields {
        text(unit, name)?;
        prop(unit, value)?;
    }
    Ok(())
}

fn prop(unit: &mut Unit, value: &TreeValue) -> Result<(), Refused> {
    match value {
        TreeValue::String(body) | TreeValue::Handle(body) | TreeValue::Site(body) => {
            text(unit, body)?
        }
        TreeValue::Binding { state, .. } => text(unit, state)?,
        TreeValue::List(values) => {
            buffer(unit, values)?;
            for value in values {
                prop(unit, value)?;
            }
        }
        TreeValue::Record(fields) => record(unit, fields)?,
        TreeValue::Node(value) => {
            unit.retain(RetainedAllocation {
                requested: std::mem::size_of::<TreeNode>() as u64,
                unused_capacity: 0,
            })?;
            node(unit, value)?;
        }
        TreeValue::Action(steps) => {
            buffer::<TreeStep>(unit, steps)?;
            for step in steps {
                buffer(unit, &step.arguments)?;
                for arg in &step.arguments {
                    prop(unit, arg)?;
                }
            }
        }
        TreeValue::Snapshot {
            record_id,
            state,
            value,
        } => {
            text(unit, record_id)?;
            text(unit, state)?;
            if let Some(value) = value {
                unit.retain(RetainedAllocation {
                    requested: std::mem::size_of::<TreeValue>() as u64,
                    unused_capacity: 0,
                })?;
                prop(unit, value)?;
            }
        }
        TreeValue::Null | TreeValue::Number(_) | TreeValue::Boolean(_) | TreeValue::Error(_) => {}
    }
    Ok(())
}

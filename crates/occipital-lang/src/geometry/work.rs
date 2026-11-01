use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TableColumn {
    pub key: String,
    pub label: String,
    pub width: Decimal,
}

/// Only cells in the viewport are prepared, never an open-page data tail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TableLayout {
    pub path: String,
    pub columns: Vec<TableColumn>,
    pub cells: Vec<Vec<crate::DisplayText>>,
    pub viewport_rows: usize,
}

pub(super) fn data(value: Option<&TreeValue>) -> Option<&TreeValue> {
    match value? {
        TreeValue::Snapshot { value, .. } => data(value.as_deref()),
        value => Some(value),
    }
}

fn items(value: Option<&TreeValue>) -> &[TreeValue] {
    match data(value) {
        Some(TreeValue::List(items)) => items,
        _ => &[],
    }
}

fn field<'a>(value: &'a TreeValue, key: &str) -> Option<&'a TreeValue> {
    match value {
        TreeValue::Record(fields) => prop(fields, key),
        _ => None,
    }
}

pub(super) fn columns(props: &Props) -> Vec<(String, String)> {
    if let Some(TreeValue::List(entries)) = prop(props, "columns") {
        return entries
            .iter()
            .take(8)
            .filter_map(|entry| match entry {
                TreeValue::String(key) => Some((key.to_string(), key.to_string())),
                TreeValue::Record(fields) => {
                    let key = text(fields, "key")?;
                    Some((
                        key.to_owned(),
                        text(fields, "label").unwrap_or(key).to_owned(),
                    ))
                }
                TreeValue::Node(node) => match &**node {
                    TreeNode::Component { props, .. } => {
                        let label = text(props, "label")?;
                        Some((label.to_owned(), label.to_owned()))
                    }
                    _ => None,
                },
                _ => None,
            })
            .collect();
    }
    match items(prop(props, "data")).first() {
        Some(TreeValue::Record(fields)) => fields
            .iter()
            .take(8)
            .map(|(key, _)| (key.to_string(), key.to_string()))
            .collect(),
        _ => Vec::new(),
    }
}

pub(super) fn chart_size(props: &Props) -> (usize, usize) {
    if has(props, "series") {
        (list(props, "labels").len(), list(props, "series").len())
    } else {
        let series = match prop(props, "y") {
            Some(TreeValue::List(items)) => items.len(),
            Some(TreeValue::String(_)) => 1,
            _ => 0,
        };
        (items(prop(props, "data")).len(), series)
    }
}

pub(super) fn markers(props: &Props) -> usize {
    items(prop(props, "places")).len()
}

/// Counts only defined catalog work terms. Callout's prose runs are deferred
/// with prose layout; no text-run, node-base or decoded-byte tariff is added.
pub(crate) fn known_work(tree: &Tree, placement: Placement) -> Result<u64, MeasureError> {
    fn add(total: &mut u64, n: u64) -> Result<(), MeasureError> {
        *total = total.checked_add(n).ok_or(NumberError::Overflow)?;
        Ok(())
    }
    fn nested(
        value: &TreeValue,
        placement: Placement,
        total: &mut u64,
    ) -> Result<(), MeasureError> {
        match value {
            TreeValue::Node(node) => add(total, node_work(node, placement)?)?,
            TreeValue::List(items) => {
                for item in items {
                    nested(item, placement, total)?;
                }
            }
            TreeValue::Record(fields) => {
                for (_, item) in fields {
                    nested(item, placement, total)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn node_work(node: &TreeNode, placement: Placement) -> Result<u64, MeasureError> {
        let mut total = 0;
        match node {
            TreeNode::Component {
                name,
                props,
                children,
            } => {
                let rows = rows(
                    props,
                    placement,
                    if name.as_str() == "LiveOutput" { 8 } else { 6 },
                )
                .integer()
                .ok_or_else(|| MeasureError::InvalidTree("displayRows".into()))?
                    as u64;
                total = match name.as_str() {
                    "Badge" | "Input" | "Slider" | "Toggle" | "Button" | "Progress" | "Callout"
                    | "Image" => 2,
                    "Styled" | "Citation" | "Card" | "InlineCode" | "Buttons" | "TabItem"
                    | "Divider" | "Slot" | "Elapsed" | "Agent" | "Model" | "OpenItem" => 1,
                    "CardHeader" | "FormControl" | "ListItem" | "Stat" => 3,
                    "Table" => 2 + columns(props).len() as u64 * (rows + 1),
                    "LinkCard" => 4,
                    "ImageGroup" => 1 + list(props, "images").len() as u64,
                    "Map" => 1 + markers(props) as u64,
                    "Chart" => {
                        let (points, series) = chart_size(props);
                        16 + points as u64 + series as u64
                    }
                    "TextContent" => {
                        1 + (text(props, "text").unwrap_or("").chars().count() as u64).div_ceil(256)
                    }
                    "CodeBlock" => {
                        2 + line_count(text(props, "codeString").unwrap_or(""))
                            .min(number(props, "maxLines", 12).integer().unwrap_or(12) as u64)
                    }
                    "Select" => 2 + list(props, "items").len() as u64,
                    "Tabs" => 1 + children.len() as u64,
                    "List" => 2 + 3 * rows,
                    "LiveOutput" => 2 + rows,
                    _ => 0,
                };
                // List's work formula already includes its visible ListItems;
                // cells beyond that viewport must not be visited again.
                if name.as_str() != "List" {
                    for child in children {
                        add(&mut total, node_work(child, placement)?)?;
                    }
                }
                for (_, value) in props {
                    nested(value, placement, &mut total)?;
                }
            }
            TreeNode::Markdown { children, .. } => {
                for child in children {
                    add(&mut total, node_work(child, placement)?)?;
                }
            }
            _ => {}
        }
        Ok(total)
    }
    let mut total = 0;
    for node in &tree.blocks {
        add(&mut total, node_work(node, placement)?)?;
    }
    Ok(total)
}

pub(super) fn line_count(text: &str) -> u64 {
    text.lines().count() as u64
}

/// Catalog clips append an ellipsis after the permitted scalar prefix.
pub(super) fn clip(text: &str, limit: usize) -> String {
    let mut chars = text.chars();
    let mut result: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        result.push('…');
    }
    result
}

fn cell(value: Option<&TreeValue>) -> crate::DisplayText {
    let text = match value {
        Some(TreeValue::String(text)) => text.to_string(),
        Some(TreeValue::Number(value)) => {
            let canonical = value.to_string();
            let (integer, fraction) = canonical.split_once('.').unwrap_or((&canonical, ""));
            let mut grouped = String::new();
            let digits = integer.strip_prefix('-').unwrap_or(integer);
            if integer.starts_with('-') {
                grouped.push('-');
            }
            for (i, c) in digits.chars().enumerate() {
                if i > 0 && (digits.len() - i).is_multiple_of(3) {
                    grouped.push(',');
                }
                grouped.push(c);
            }
            if !fraction.is_empty() {
                grouped.push('.');
                grouped.push_str(fraction);
            }
            grouped
        }
        Some(TreeValue::Boolean(value)) => value.to_string(),
        Some(TreeValue::Null) | None => String::new(),
        Some(TreeValue::Error(code)) => code.as_str().to_owned(),
        _ => "…".to_owned(),
    };
    crate::DisplayText::new(&clip(&text, 256))
}

impl<M: Metrics> Engine<'_, M> {
    pub(super) fn table(
        &mut self,
        props: &Props,
        width: Decimal,
        path: &str,
    ) -> Result<(), LayoutError> {
        let mut columns = columns(props)
            .into_iter()
            .map(|(key, label)| {
                let w = self
                    .tw(&clip(&label, 256), TextStyle::Subheadline)?
                    .checked_add(d(16))?
                    .max(d(64));
                Ok(TableColumn {
                    key,
                    label,
                    width: w,
                })
            })
            .collect::<Result<Vec<_>, LayoutError>>()?;
        let used = columns
            .iter()
            .try_fold(Decimal::ZERO, |sum, c| sum.checked_add(c.width))?;
        if !columns.is_empty() && used < width {
            let extra = width
                .checked_sub(used)?
                .checked_div(d(columns.len() as i128))?;
            for column in &mut columns {
                column.width = column.width.checked_add(extra)?;
            }
        }
        let viewport_rows = rows(props, self.context.placement, 6)
            .integer()
            .ok_or_else(|| MeasureError::InvalidTree("displayRows".into()))?
            as usize;
        let descriptors = list(props, "columns");
        let column_form = descriptors
            .first()
            .is_some_and(|v| matches!(v, TreeValue::Node(_)));
        let cells = if column_form {
            let count = descriptors
                .iter()
                .filter_map(|value| match value {
                    TreeValue::Node(node) => match &**node {
                        TreeNode::Component { props, .. } => Some(list(props, "data").len()),
                        _ => None,
                    },
                    _ => None,
                })
                .max()
                .unwrap_or(0)
                .min(viewport_rows);
            (0..count)
                .map(|row| {
                    descriptors
                        .iter()
                        .take(8)
                        .map(|value| {
                            let value = match value {
                                TreeValue::Node(node) => match &**node {
                                    TreeNode::Component { props, .. } => {
                                        list(props, "data").get(row)
                                    }
                                    _ => None,
                                },
                                _ => None,
                            };
                            cell(value)
                        })
                        .collect()
                })
                .collect()
        } else {
            items(prop(props, "data"))
                .iter()
                .take(viewport_rows)
                .map(|row| columns.iter().map(|c| cell(field(row, &c.key))).collect())
                .collect()
        };
        self.tables.push(TableLayout {
            path: path.to_owned(),
            columns,
            cells,
            viewport_rows,
        });
        Ok(())
    }
}

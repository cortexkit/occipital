use super::*;
use TextStyle::*;

impl<M: Metrics> Engine<'_, M> {
    pub(super) fn l(&mut self, style: TextStyle) -> ResultH {
        if let Some(value) = self.lines.get(&style) {
            return Ok(*value);
        }
        let value = valid(self.metrics.l(style))?;
        self.lines.insert(style, value);
        Ok(value)
    }

    fn t(
        &self,
        value: Option<&str>,
        style: TextStyle,
        width: Decimal,
        lines: Option<u32>,
    ) -> ResultH {
        match value {
            None => Ok(Decimal::ZERO),
            Some(value) => Ok(valid(self.metrics.t(value, style, width, lines))?),
        }
    }

    pub(super) fn tw(&self, value: &str, style: TextStyle) -> ResultH {
        Ok(valid(self.metrics.tw(value, style))?)
    }

    pub(super) fn stack(
        &mut self,
        nodes: &[TreeNode],
        width: Decimal,
        path: &str,
        gap: Decimal,
    ) -> ResultH {
        let mut height = Decimal::ZERO;
        for (i, node) in nodes.iter().enumerate() {
            if i > 0 {
                height = height.checked_add(gap)?;
            }
            height = height.checked_add(self.node(node, width, &format!("{path}/{i}"))?)?;
        }
        Ok(height)
    }

    fn node(&mut self, node: &TreeNode, width: Decimal, path: &str) -> ResultH {
        let (height, box_width, state) = match node {
            TreeNode::Component {
                name,
                props,
                children,
            } => {
                let state = render_state(name.as_str(), props);
                let h = self.component(name.as_str(), props, children, width, path)?;
                let w = self.fixed_width(node, width)?.unwrap_or(width).ceil()?;
                (h, w, state)
            }
            TreeNode::ErrorCell { code, .. } => (
                d(16)
                    .checked_add(self.l(Footnote)?)?
                    .checked_add(self.l(Caption1)?)?,
                width,
                RenderState::Error(*code),
            ),
            TreeNode::ErrorMarker(code) => (
                self.l(Caption1)?.checked_add(d(4))?,
                self.tw(code.as_str(), Caption1)?
                    .checked_add(d(8))?
                    .ceil()?,
                RenderState::Error(*code),
            ),
            TreeNode::Reference(label) => (
                self.l(Caption1)?.checked_add(d(4))?,
                self.tw(&work::clip(label.as_str(), 24), Caption1)?
                    .checked_add(d(8))?
                    .min(width)
                    .ceil()?,
                RenderState::Ready,
            ),
            TreeNode::Markdown {
                kind: crate::eval::tree::MarkdownKind::Task(_),
                ..
            } => {
                let h = self.l(Body)?;
                (h, h.ceil()?, RenderState::Ready)
            }
            TreeNode::Markdown { .. } | TreeNode::Text(_) => return Err(LayoutError::Prose),
        };
        let height = valid(height)?.ceil()?;
        self.boxes.push(MeasuredBox {
            path: path.to_owned(),
            height,
            width: box_width,
            state,
        });
        Ok(height)
    }

    fn component(
        &mut self,
        name: &str,
        p: &Props,
        children: &[TreeNode],
        w: Decimal,
        path: &str,
    ) -> ResultH {
        let clipped = clipped_props(name, p);
        let p = &clipped;
        Ok(match name {
            "Badge" | "Citation" | "ReferenceChip" | "ErrorMarker" => {
                self.l(Caption1)?.checked_add(d(4))?
            }
            "InlineCode" => self.l(MonoFootnote)?.checked_add(d(4))?,
            "Slot" => {
                // The content is measured so the app has its boxes, but the
                // Slot keeps its own height: whatever doesn't fit is clipped,
                // so the row never changes height when its data arrives.
                // Prose content is laid out by the app, inside the same box.
                let inner = self.slot_width(p, w)?.unwrap_or(w);
                match self.stack(children, inner, path, Decimal::ZERO) {
                    Ok(_) | Err(LayoutError::Prose) => {}
                    Err(error) => return Err(error),
                }
                number(p, "lines", 1).checked_mul(self.l(Body)?)?
            }
            // Drawn by the app at one body line: the clock, the creature and
            // name, and the model's name are each cut to their fixed width.
            "Elapsed" | "Agent" | "Model" => self.l(Body)?,
            // A link or plain text in the page's prose, measured by the app.
            "Styled" | "Markdown" | "OpenItem" => return Err(LayoutError::Prose),
            "GenericToolRow" => return Err(MeasureError::GenericToolRow.into()),
            "Col" | "Series" | "SelectItem" => Decimal::ZERO,
            "TaskCheckbox" => self.l(Body)?,
            "Callout" => {
                let cw = w.checked_sub(d(52))?;
                let mut h = d(24);
                if has(p, "title") {
                    h = h
                        .checked_add(self.t(text(p, "title"), Headline, cw, Some(3))?)?
                        .checked_add(d(4))?;
                }
                if !children.is_empty() {
                    return Err(LayoutError::Prose);
                }
                h.checked_add(self.t(text(p, "description"), Body, cw, None)?)?
            }
            "Table" => {
                self.table(p, w, path)?;
                rows(p, self.context.placement, 6)
                    .checked_add(d(1))?
                    .checked_mul(self.l(Subheadline)?.checked_add(d(12))?)?
                    .checked_add(self.foot()?)?
            }
            "LinkCard" => {
                if self.context.network == Network::No {
                    let form = format!(
                        "{} ({})",
                        text(p, "title").unwrap_or(""),
                        text(p, "href").unwrap_or("")
                    );
                    self.t(Some(&form), Body, w, None)?
                } else {
                    let cw = w.checked_sub(d(24))?;
                    let mut h =
                        d(24).checked_add(self.t(text(p, "title"), Headline, cw, Some(2))?)?;
                    if has(p, "description") {
                        h = h.checked_add(d(2))?.checked_add(self.t(
                            text(p, "description"),
                            Subheadline,
                            cw,
                            Some(2),
                        )?)?;
                    }
                    h.checked_add(d(2))?.checked_add(self.l(Caption1)?)?
                }
            }
            "Image" | "Map" => {
                let default = if name == "Image" { "4:3" } else { "16:9" };
                let mut h = w.checked_mul(aspect(p, default))?;
                if has(p, "caption") {
                    h = h.checked_add(d(4))?.checked_add(self.t(
                        text(p, "caption"),
                        Footnote,
                        w,
                        Some(2),
                    )?)?;
                }
                h
            }
            "ImageGroup" => {
                let n = list(p, "images").len() as i128;
                if n == 0 {
                    return Err(MeasureError::InvalidTree("ImageGroup.images".into()).into());
                }
                let cols = d(n.min(2));
                let rows = d((n + 1) / 2);
                let cw = w
                    .checked_sub(d(2).checked_mul(cols.checked_sub(d(1))?)?)?
                    .checked_div(cols)?;
                let mut h = rows
                    .checked_mul(cw)?
                    .checked_mul(aspect(p, "4:3"))?
                    .checked_add(rows.checked_sub(d(1))?.checked_mul(d(2))?)?;
                if has(p, "caption") {
                    h = h.checked_add(d(4))?.checked_add(self.t(
                        text(p, "caption"),
                        Footnote,
                        w,
                        Some(2),
                    )?)?;
                }
                h
            }
            "Chart" => d(120).max(w.checked_mul(aspect(p, "16:9"))?),
            "Card" => self.card(p, children, w, path)?,
            "CardHeader" => {
                let mut h = self.t(text(p, "title"), Title3, w, Some(2))?;
                if has(p, "subtitle") {
                    h = h.checked_add(d(2))?.checked_add(self.t(
                        text(p, "subtitle"),
                        Subheadline,
                        w,
                        Some(2),
                    )?)?;
                }
                h
            }
            "TextContent" => {
                let style = match text(p, "size").unwrap_or("default") {
                    "small" => Footnote,
                    "large" => Title3,
                    _ => Body,
                };
                self.t(
                    text(p, "text"),
                    style,
                    w,
                    Some(number(p, "maxLines", 24).integer().unwrap_or(24) as u32),
                )?
            }
            "CodeBlock" => {
                let lines = d(work::line_count(text(p, "codeString").unwrap_or("")) as i128);
                let max = number(p, "maxLines", 12);
                let mut h = d(20);
                if has(p, "language") {
                    h = h.checked_add(self.l(Caption1)?)?.checked_add(d(2))?;
                }
                h = h.checked_add(lines.min(max).checked_mul(self.l(Mono)?)?)?;
                if lines > max {
                    h = h.checked_add(d(2))?.checked_add(self.l(Footnote)?)?;
                }
                h
            }
            "FormControl" => {
                let input = match prop(p, "input") {
                    Some(TreeValue::Node(node)) => node,
                    _ => return Err(MeasureError::InvalidTree("FormControl.input".into()).into()),
                };
                let mut h = self
                    .t(text(p, "label"), Subheadline, w, Some(2))?
                    .checked_add(d(2))?
                    .checked_add(self.node(input, w, &format!("{path}/input"))?)?;
                if has(p, "hint") {
                    h = h.checked_add(d(2))?.checked_add(self.t(
                        text(p, "hint"),
                        Footnote,
                        w,
                        Some(2),
                    )?)?;
                }
                h
            }
            "Input" => {
                let lines = if text(p, "type") == Some("number") {
                    d(1)
                } else {
                    number(p, "lines", 1)
                };
                d(44).max(lines.checked_mul(self.l(Body)?)?.checked_add(d(20))?)
            }
            "Select" | "Slider" | "Button" => d(44),
            "Toggle" => k("tap")?.max(
                self.t(
                    text(p, "label"),
                    Body,
                    w.checked_sub(k("switchW")?)?.checked_sub(k("gap.m")?)?,
                    Some(2),
                )?
                .checked_add(d(2).checked_mul(k("fieldPadV")?)?)?,
            ),
            "Buttons" => {
                let buttons: Vec<&TreeNode> = list(p, "buttons")
                    .iter()
                    .filter_map(|v| match v {
                        TreeValue::Node(node) => Some(&**node),
                        _ => None,
                    })
                    .collect();
                let lines = if text(p, "direction") == Some("column") {
                    buttons.len()
                } else {
                    self.button_lines(&buttons, w)?
                };
                for (i, button) in buttons.iter().enumerate() {
                    self.node(button, w, &format!("{path}/buttons/{i}"))?;
                }
                d(lines as i128)
                    .checked_mul(k("tap")?)?
                    .checked_add(d(lines.saturating_sub(1) as i128).checked_mul(k("gap.s")?)?)?
            }
            "Tabs" => {
                let mut max = Decimal::ZERO;
                for (i, panel) in children.iter().enumerate() {
                    max = max.max(self.node(panel, w, &format!("{path}/{i}"))?);
                }
                d(48).checked_add(max)?
            }
            "TabItem" => self.stack(children, w, path, k("gap.m")?)?,
            "List" => rows(p, self.context.placement, 6)
                .checked_mul(self.item()?)?
                .checked_add(self.foot()?)?,
            "ListItem" => self.item()?,
            "LiveOutput" => d(20)
                .checked_add(
                    rows(p, self.context.placement, 8).checked_mul(self.l(MonoFootnote)?)?,
                )?
                .checked_add(self.foot()?)?,
            "Stat" => {
                let mut h = self
                    .l(Footnote)?
                    .checked_add(d(2))?
                    .checked_add(self.l(Title2)?)?;
                if has(p, "caption") {
                    h = h.checked_add(d(2))?.checked_add(self.l(Caption1)?)?;
                }
                h
            }
            "Progress" => {
                if has(p, "label") {
                    self.l(Footnote)?.checked_add(d(8))?
                } else {
                    d(6)
                }
            }
            "Divider" => d(17),
            "ErrorCell" => d(16)
                .checked_add(self.l(Footnote)?)?
                .checked_add(self.l(Caption1)?)?,
            "LoadingRoot" => d(28)
                .checked_add(self.l(Headline)?)?
                .checked_add(self.l(Subheadline)?)?,
            "RowFrame" => {
                let first = self
                    .first
                    .ok_or_else(|| MeasureError::InvalidTree("RowFrame.first".into()))?;
                let citations = std::mem::replace(&mut self.citations, citation_ids(first));
                let height = self.stack(&first.blocks, w, &format!("{path}/first"), k("gap.m")?);
                self.citations = citations;
                let h = height?;
                h.min(k("rowCapMin")?.max(d(16).checked_mul(self.l(Body)?)?))
                    .checked_add(k("tap")?)?
            }
            "ProgramFallback" => d(24)
                .checked_add(self.t(text(p, "fallback"), Body, w.checked_sub(d(24))?, None)?)?
                .checked_add(d(52))?,
            "ArgumentsShortened" => self.l(Caption1)?.checked_add(d(2))?,
            "ReplyShortened" => self.l(Footnote)?.checked_add(d(8))?,
            _ => return Err(MeasureError::InvalidTree(name.to_owned()).into()),
        })
    }

    fn foot(&mut self) -> ResultH {
        Ok(self.l(Footnote)?.checked_add(d(12))?)
    }
    fn item(&mut self) -> ResultH {
        Ok(d(16)
            .checked_add(self.l(Body)?)?
            .checked_add(self.l(Footnote)?)?)
    }

    /// The one-based index of this citation among the tree's citations,
    /// which are all collected before layout. A citation missing from that
    /// list is a tree the formulas cannot measure.
    fn citation_index(&self, p: &Props) -> Result<usize, MeasureError> {
        let identity = citation_identity(p);
        self.citations
            .iter()
            .position(|key| *key == identity)
            .map(|index| index + 1)
            .ok_or_else(|| MeasureError::InvalidTree("Citation.source".into()))
    }

    fn fixed_width(&mut self, node: &TreeNode, w: Decimal) -> Result<Option<Decimal>, LayoutError> {
        let TreeNode::Component {
            name,
            props: p,
            children,
        } = node
        else {
            return Ok(None);
        };
        let clipped = clipped_props(name.as_str(), p);
        let p = &clipped;
        let width = match name.as_str() {
            "Badge" => {
                let label = text(p, "text").map(str::to_owned).unwrap_or_else(|| {
                    children
                        .iter()
                        .filter_map(|n| match n {
                            TreeNode::Text(t) => Some(t.as_str()),
                            _ => None,
                        })
                        .collect()
                });
                self.tw(&work::clip(&label, 64), Caption1)?
                    .checked_add(d(12))?
            }
            "Citation" => {
                let index = self.citation_index(p)?.to_string();
                self.tw(&index, Caption1)?.checked_add(d(8))?
            }
            "InlineCode" => self
                .tw(
                    &work::clip(text(p, "code").unwrap_or(""), 256),
                    MonoFootnote,
                )?
                .checked_add(d(8))?,
            "Button" => self
                .tw(text(p, "label").unwrap_or(""), Headline)?
                .checked_add(d(2).checked_mul(k("buttonPadH")?)?)?,
            "Toggle" => self
                .tw(text(p, "label").unwrap_or(""), Body)?
                .checked_add(k("switchW")?)?
                .checked_add(k("gap.m")?)?,
            "Buttons" => {
                let mut sum = Decimal::ZERO;
                let mut max = Decimal::ZERO;
                for value in list(p, "buttons") {
                    if let TreeValue::Node(button) = value {
                        let width = self.fixed_width(button, w)?.unwrap_or(w);
                        sum = sum.checked_add(width)?;
                        max = max.max(width);
                    }
                }
                if text(p, "direction") == Some("column") {
                    max
                } else {
                    sum.checked_add(
                        k("gap.s")?
                            .checked_mul(d(list(p, "buttons").len().saturating_sub(1) as i128))?,
                    )?
                }
            }
            "ReferenceChip" => self
                .tw(&work::clip(text(p, "label").unwrap_or(""), 24), Caption1)?
                .checked_add(d(8))?,
            "ErrorMarker" => self
                .tw(text(p, "code").unwrap_or(""), Caption1)?
                .checked_add(d(8))?,
            "TaskCheckbox" => self.l(Body)?,
            "Elapsed" => self.tw("0:00:00", Body)?,
            "Agent" => self
                .l(Body)?
                .checked_add(k("gap.s")?)?
                .checked_add(self.tw(&"0".repeat(16), Body)?)?,
            "Model" => self.tw(&"0".repeat(24), Body)?,
            "Slot" => match self.slot_width(p, w)? {
                Some(width) => width,
                None => return Ok(None),
            },
            _ => return Ok(None),
        };
        let width = if matches!(name.as_str(), "Citation" | "ErrorMarker" | "TaskCheckbox") {
            width
        } else {
            width.min(w)
        };
        Ok(Some(width.ceil()?))
    }

    /// A Slot's width class as a fixed width, counted in the digit zero of the
    /// body style the way a CSS `ch` unit is, so it follows the person's text
    /// size; `None` for `fill`, which takes what the row leaves.
    fn slot_width(&mut self, p: &Props, w: Decimal) -> Result<Option<Decimal>, LayoutError> {
        let zeros = match text(p, "width") {
            Some("narrow") => 8,
            Some("short") => 12,
            Some("medium") => 24,
            Some("long") => 40,
            _ => return Ok(None),
        };
        Ok(Some(self.tw(&"0".repeat(zeros), Body)?.min(w)))
    }

    fn button_lines(&mut self, buttons: &[&TreeNode], w: Decimal) -> Result<usize, LayoutError> {
        let mut lines = 0;
        let mut used = Decimal::ZERO;
        for button in buttons {
            let width = self.fixed_width(button, w)?.unwrap_or(w);
            let next = if used == Decimal::ZERO {
                width
            } else {
                used.checked_add(k("gap.s")?)?.checked_add(width)?
            };
            if lines == 0 {
                lines = 1;
            }
            if used > Decimal::ZERO && next > w {
                lines += 1;
                used = width;
            } else {
                used = next;
            }
        }
        Ok(lines)
    }

    fn card(&mut self, p: &Props, children: &[TreeNode], w: Decimal, path: &str) -> ResultH {
        let pad = if text(p, "appearance").unwrap_or("card") == "card" {
            k("cardPad")?
        } else {
            Decimal::ZERO
        };
        let padding = pad.checked_mul(d(2))?;
        let cw = w.checked_sub(padding)?.max(Decimal::ZERO);
        let gap = k(&format!("gap.{}", text(p, "gap").unwrap_or("m")))?;
        if text(p, "direction").unwrap_or("column") == "column" {
            return Ok(padding.checked_add(self.stack(children, cw, path, gap)?)?);
        }
        let wrap = matches!(prop(p, "wrap"), Some(TreeValue::Boolean(true)));
        let widths = children
            .iter()
            .map(|child| {
                self.fixed_width(child, cw)
                    .map(|width| width.map(|width| width.min(cw)))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if !wrap {
            let gaps = gap.checked_mul(d(children.len().saturating_sub(1) as i128))?;
            let mut left = cw.checked_sub(gaps)?.max(Decimal::ZERO);
            let mut assigned: Vec<Option<Decimal>> = vec![None; children.len()];
            // A Slot's box is the space its value fills in later, so it is
            // reserved before any other fixed child: a long code or label
            // beside it is cut to what the Slots leave, never the Slot.
            let is_slot = |node: &TreeNode| matches!(node, TreeNode::Component { name, .. } if name.as_str() == "Slot");
            let slots = (0..children.len()).filter(|&i| is_slot(&children[i]));
            let others = (0..children.len()).filter(|&i| !is_slot(&children[i]));
            for i in slots.chain(others) {
                if let Some(width) = widths[i] {
                    let width = width.min(left);
                    assigned[i] = Some(width);
                    left = left.checked_sub(width)?;
                }
            }
            let flexible = widths.iter().filter(|w| w.is_none()).count();
            let share = if flexible == 0 {
                Decimal::ZERO
            } else {
                left.checked_div(d(flexible as i128))?.max(k("flexMin")?)
            };
            let mut height = Decimal::ZERO;
            for (i, child) in children.iter().enumerate() {
                height = height.max(self.row_child(
                    child,
                    assigned[i].unwrap_or(share),
                    &format!("{path}/{i}"),
                    widths[i].is_some(),
                )?);
            }
            return Ok(padding.checked_add(height)?);
        }
        let mut line_used = Decimal::ZERO;
        let mut line_height = Decimal::ZERO;
        let mut line_count: usize = 0;
        let mut total = Decimal::ZERO;
        let mut in_line = false;
        for (i, child) in children.iter().enumerate() {
            let available = cw.checked_sub(line_used)?.checked_sub(if in_line {
                gap
            } else {
                Decimal::ZERO
            })?;
            let needed = widths[i].unwrap_or(k("flexMin")?);
            if in_line && needed > available {
                total = total.checked_add(line_height)?;
                line_count += 1;
                line_used = Decimal::ZERO;
                line_height = Decimal::ZERO;
                in_line = false;
            }
            let available = cw.checked_sub(line_used)?.checked_sub(if in_line {
                gap
            } else {
                Decimal::ZERO
            })?;
            let width = widths[i].unwrap_or(available.max(k("flexMin")?));
            line_height = line_height.max(self.row_child(
                child,
                width,
                &format!("{path}/{i}"),
                widths[i].is_some(),
            )?);
            line_used = line_used
                .checked_add(if in_line { gap } else { Decimal::ZERO })?
                .checked_add(width)?;
            in_line = true;
            if widths[i].is_none() {
                total = total.checked_add(line_height)?;
                line_count += 1;
                line_used = Decimal::ZERO;
                line_height = Decimal::ZERO;
                in_line = false;
            }
        }
        if in_line {
            total = total.checked_add(line_height)?;
            line_count += 1;
        }
        Ok(padding
            .checked_add(total)?
            .checked_add(gap.checked_mul(d(line_count.saturating_sub(1) as i128))?)?)
    }

    fn row_child(&mut self, child: &TreeNode, width: Decimal, path: &str, fixed: bool) -> ResultH {
        let height = self.node(child, width, path)?;
        if fixed {
            // Card clips fixed boxes to the space it assigned, even when the
            // component's standalone width formula has no min(W, ...).
            // Measuring a component always emits its box, so there is one.
            if let Some(measured) = self.boxes.last_mut() {
                measured.width = measured.width.min(width);
            }
        }
        Ok(height)
    }
}

fn clipped_props<'a>(name: &str, props: &'a Props) -> std::borrow::Cow<'a, Props> {
    let catalog = crate::catalog::Catalog::tool_templates();
    let needs_clip = props.iter().any(|(key, value)| {
        let limit = catalog.components.get(name)
            .and_then(|component| component.props.get(key.as_str()))
            .and_then(|prop| prop.rules.get("clip"))
            .and_then(serde_json::Value::as_str)
            .and_then(|limit| catalog.clip_limit(limit));
        matches!((value, limit), (TreeValue::String(text), Some(limit)) if text.as_str().chars().count() > limit)
    });
    if !needs_clip {
        return std::borrow::Cow::Borrowed(props);
    }
    std::borrow::Cow::Owned(
        props
            .iter()
            .map(|(key, value)| {
                let limit = catalog
                    .components
                    .get(name)
                    .and_then(|component| component.props.get(key.as_str()))
                    .and_then(|prop| prop.rules.get("clip"))
                    .and_then(serde_json::Value::as_str)
                    .and_then(|limit| catalog.clip_limit(limit));
                let value = match (value, limit) {
                    (TreeValue::String(text), Some(limit)) => TreeValue::String(
                        crate::DisplayText::new(&work::clip(text.as_str(), limit)),
                    ),
                    _ => value.clone(),
                };
                (key.clone(), value)
            })
            .collect(),
    )
}

pub(crate) fn render_state(name: &str, props: &Props) -> RenderState {
    if name == "Chart" {
        let (points, series) = work::chart_size(props);
        if points > 2048 || series > 8 {
            return RenderState::Error(DiagnosticCode::RenderLimit);
        }
    }
    if name == "Map" && work::markers(props) > 100 {
        return RenderState::Error(DiagnosticCode::RenderLimit);
    }
    let input = match name {
        "Table" | "Chart" => "data",
        "Map" => "places",
        "Image" => "image",
        "Citation" | "ReferenceChip" => "source",
        _ => "",
    };
    match prop(props, input) {
        Some(TreeValue::Error(code)) => RenderState::Error(*code),
        Some(TreeValue::Snapshot {
            value: Some(value), ..
        }) if matches!(value.as_ref(), TreeValue::Error(_)) => match value.as_ref() {
            TreeValue::Error(code) => RenderState::Error(*code),
            // The guard above admits only an error, so this arm never runs;
            // it is here so the match needs no panic.
            _ => RenderState::Unavailable,
        },
        Some(TreeValue::Snapshot { state, value, .. }) => match state.as_str() {
            "held" => RenderState::Held,
            "loading" | "pending" => RenderState::Loading,
            "unavailable" | "unresolved" => RenderState::Unavailable,
            _ if name == "Map" && (state.as_str() != "resolved" || value.is_none()) => {
                RenderState::Unavailable
            }
            _ => RenderState::Ready,
        },
        _ if name == "Map" => RenderState::Unavailable,
        _ => RenderState::Ready,
    }
}

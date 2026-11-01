use super::*;

/// Every function name the checker accepts, in §6 order. A call to any other
/// name is `E_UNKNOWN_FUNCTION`, and the public function-table export lists
/// exactly these names, so the export cannot drift from the checker.
pub(crate) const FUNCTIONS: [&str; 53] = [
    "Count",
    "Sum",
    "Avg",
    "First",
    "Last",
    "Min",
    "Max",
    "Filter",
    "Sort",
    "Each",
    "Slice",
    "Range",
    "Contains",
    "Keys",
    "Values",
    "Join",
    "IsError",
    "Round",
    "Floor",
    "Ceil",
    "Abs",
    "Sqrt",
    "Clamp",
    "Div",
    "Number",
    "Text",
    "Len",
    "Lines",
    "Upper",
    "Lower",
    "Trim",
    "StartsWith",
    "EndsWith",
    "Split",
    "Replace",
    "Truncate",
    "FormatNumber",
    "FormatBytes",
    "FormatDuration",
    "FormatDate",
    "FormatRelative",
    "ParseDate",
    "Parse",
    "Query",
    "Watch",
    "PagedList",
    "Action",
    "Set",
    "Reset",
    "Refresh",
    "LoadMore",
    "Compose",
    "Open",
];

/// The reserved write names: refused with `E_WRITE_NOT_ALLOWED` in every
/// profile, never v1 constructors or steps (§2.3, §7.4).
pub(crate) const RESERVED_WRITES: [&str; 2] = ["Mutation", "Run"];

/// The site constructors (§8.3–§8.6). Only profiles that allow sites accept
/// them (§2.3), and the profile export lists them per profile.
pub(crate) const SITE_CONSTRUCTORS: [&str; 3] = ["Query", "Watch", "PagedList"];

/// One row of the function-table export: the arity and the signature text
/// the checker quotes in its diagnostics (§12.1 `signature`).
pub(crate) struct FunctionEntry {
    pub name: &'static str,
    pub min: usize,
    /// `None` for a variadic function such as `Min` and `Max`.
    pub max: Option<usize>,
    pub signature: String,
}

/// The checker's own function table, in `FUNCTIONS` order. Every listed
/// function has a signature; the export test compares the whole table.
pub(crate) fn function_table() -> Vec<FunctionEntry> {
    FUNCTIONS
        .iter()
        .filter_map(|name| {
            let signature = signature(name)?;
            Some(FunctionEntry {
                name,
                min: signature.min,
                max: (signature.max != usize::MAX).then_some(signature.max),
                signature: signature_text(name, &signature),
            })
        })
        .collect()
}

/// A shared signature such as `(x: number) -> number` is written with the
/// function's name in front, as §6 writes it.
fn signature_text(name: &str, signature: &Signature) -> String {
    if signature.text.starts_with('(') {
        format!("{name}{}", signature.text)
    } else {
        signature.text.to_owned()
    }
}

struct Signature {
    min: usize,
    max: usize,
    parameters: &'static [u16],
    text: &'static str,
}

fn signature(name: &str) -> Option<Signature> {
    let (min, max, parameters, text): (_, _, &'static [u16], _) = match name {
        "Count" | "Sum" | "Avg" | "First" | "Last" => {
            (1, 1, &[LIST | ABSENT], "(list: list | absent)")
        }
        "Min" | "Max" => (
            1,
            usize::MAX,
            &[NUMBER | STRING | ABSENT | LIST],
            "(list) or (a: number | string, b: number | string, ...)",
        ),
        "Filter" => (
            4,
            4,
            &[LIST | ABSENT, STRING | ABSENT, STRING, ANY],
            "Filter(list, field: string | null, op: string, value) -> list",
        ),
        "Sort" => (
            1,
            3,
            &[LIST | ABSENT, STRING | ABSENT, STRING],
            "Sort(list, field?: string | null, direction?: string) -> list",
        ),
        "Each" => (
            3,
            3,
            &[LIST | ABSENT, STRING, ANY],
            "Each(list, \"name\", template) -> list",
        ),
        "Slice" => (
            2,
            3,
            &[LIST | STRING | ABSENT, NUMBER, NUMBER],
            "Slice(x: list | string | absent, start: number, end?: number)",
        ),
        "Range" => (
            2,
            3,
            &[NUMBER, NUMBER, NUMBER],
            "Range(start: number, end: number, step?: number) -> list<number>",
        ),
        "Contains" => (
            2,
            2,
            &[LIST | STRING | ABSENT, ANY],
            "Contains(x: list | string | absent, y) -> boolean",
        ),
        "Keys" | "Values" => (
            1,
            1,
            &[RECORD | ABSENT],
            "(record: record | absent) -> list",
        ),
        "Join" => (
            2,
            2,
            &[LIST | ABSENT, STRING],
            "Join(list, separator: string) -> string",
        ),
        "IsError" => (1, 1, &[ANY], "IsError(x) -> boolean"),
        "Round" => (
            1,
            2,
            &[NUMBER, NUMBER],
            "Round(x: number, digits?: number) -> number",
        ),
        "Floor" | "Ceil" | "Abs" | "Sqrt" => (1, 1, &[NUMBER], "(x: number) -> number"),
        "Clamp" => (
            3,
            3,
            &[NUMBER, NUMBER, NUMBER],
            "Clamp(x: number, low: number, high: number) -> number",
        ),
        "Div" => (
            2,
            2,
            &[NUMBER, NUMBER],
            "Div(a: number, b: number) -> number",
        ),
        "Number" => (
            1,
            1,
            &[NUMBER | STRING | ABSENT],
            "Number(x: number | string | absent) -> number | null",
        ),
        "Text" => (1, 1, &[SCALAR], "Text(x: scalar) -> string"),
        "Len" | "Lines" => (1, 1, &[STRING | ABSENT], "(s: string | absent)"),
        "Upper" | "Lower" | "Trim" => (1, 1, &[STRING], "(s: string) -> string"),
        "StartsWith" | "EndsWith" | "Split" => (
            2,
            2,
            &[STRING | ABSENT, STRING],
            "(s: string | absent, pattern: string)",
        ),
        "Replace" => (
            3,
            3,
            &[STRING, STRING, STRING],
            "Replace(s: string, from: string, to: string) -> string",
        ),
        "Truncate" => (
            2,
            2,
            &[STRING, NUMBER],
            "Truncate(s: string, max: number) -> string",
        ),
        "FormatNumber" => (
            1,
            3,
            &[NUMBER, NUMBER | ABSENT, BOOLEAN],
            "FormatNumber(x: number, decimals?: number | null, grouping?: boolean) -> string",
        ),
        "FormatBytes" => (
            1,
            2,
            &[NUMBER, NUMBER],
            "FormatBytes(n: number, decimals?: number) -> string",
        ),
        "FormatDuration" => (
            1,
            2,
            &[NUMBER, STRING],
            "FormatDuration(ms: number, style?: string) -> string",
        ),
        "FormatDate" => (
            1,
            3,
            &[NUMBER, STRING, STRING],
            "FormatDate(ms: number, pattern?: string, zone?: string) -> string",
        ),
        "FormatRelative" => (
            2,
            2,
            &[NUMBER, NUMBER],
            "FormatRelative(time: number, now: number) -> string",
        ),
        "ParseDate" => (1, 1, &[STRING], "ParseDate(s: string) -> number"),
        "Parse" => (
            2,
            3,
            &[SNAPSHOT, STRING, RECORD],
            "Parse(snapshot, format: string, options?: record)",
        ),
        "Query" => (
            3,
            4,
            &[
                STRING,
                SCALAR | LIST | RECORD,
                SCALAR | LIST | RECORD,
                NUMBER,
            ],
            "Query(op: string, args: data, default: data, refreshSeconds?: number)",
        ),
        "Watch" => (
            2,
            2,
            &[STRING, SCALAR | LIST | RECORD],
            "Watch(op: string, args: data)",
        ),
        "PagedList" => (
            2,
            3,
            &[STRING, SCALAR | LIST | RECORD, NUMBER],
            "PagedList(op: string, args: data, pageSize?: number)",
        ),
        "Action" => (1, 1, &[LIST], "Action(steps: list<action>) -> action"),
        "Set" => (
            2,
            2,
            &[SCALAR | LIST | RECORD, SCALAR | LIST | RECORD],
            "Set($state, value: data) -> action",
        ),
        "Reset" => (1, 1, &[SCALAR | LIST | RECORD], "Reset($state) -> action"),
        "Refresh" | "LoadMore" => (1, 1, &[ENVELOPE], "(site) -> action"),
        "Compose" => (1, 1, &[STRING], "Compose(text: string) -> action"),
        "Open" => (
            1,
            1,
            &[STRING | HANDLE],
            "Open(target: string | handle) -> action",
        ),
        _ => return None,
    };
    Some(Signature {
        min,
        max,
        parameters,
        text,
    })
}

impl Checker<'_> {
    pub(super) fn call(
        &mut self,
        name: &str,
        node: &Node,
        binders: &mut Vec<(String, Type)>,
    ) -> Type {
        if RESERVED_WRITES.contains(&name) {
            self.report(Code::WriteNotAllowed, node, name, None);
            return Type::Unknown;
        }
        let Some(signature) = FUNCTIONS.contains(&name).then(|| signature(name)).flatten() else {
            self.report(Code::UnknownFunction, node, name, None);
            for child in &node.children {
                self.infer(child, binders);
            }
            return Type::Unknown;
        };
        let signature_text = signature_text(name, &signature);
        if node.children.len() < signature.min || node.children.len() > signature.max {
            self.report(Code::Arity, node, name, Some(&signature_text));
            for child in &node.children {
                self.infer(child, binders);
            }
            return Type::Unknown;
        }
        let mut args = vec![];
        for (index, child) in node.children.iter().enumerate() {
            let pushed = if name == "Each" && index == 2 {
                if let Kind::Expression(E::String(binder)) = &node.children[1].kind {
                    let item = match &args[0] {
                        Type::List(item) => *item.clone(),
                        _ => Type::Unknown,
                    };
                    binders.push((binder.clone(), item));
                    true
                } else {
                    false
                }
            } else {
                false
            };
            let ty = self.infer(child, binders);
            if pushed {
                binders.pop();
            }
            let expected = signature.parameters[index.min(signature.parameters.len() - 1)];
            self.require(&ty, expected, child, name, Some(&signature_text));
            args.push(ty);
        }
        if matches!(name, "Set" | "Reset")
            && !matches!(node.children[0].kind, Kind::Expression(E::State(_)))
        {
            self.report(
                Code::LiteralRequired,
                &node.children[0],
                name,
                Some(&signature_text),
            );
        }
        if name == "Set" {
            self.require(
                &args[1],
                args[0].mask(),
                &node.children[1],
                name,
                Some(&signature_text),
            );
        }
        if name == "LoadMore"
            && matches!(
                args[0],
                Type::Envelope(SiteKind::Query | SiteKind::Watch, _)
            )
        {
            self.report(
                Code::ArgType,
                &node.children[0],
                name,
                Some(&signature_text),
            );
        }
        if name == "Contains" && args[0] == Type::String {
            self.require(
                &args[1],
                STRING,
                &node.children[1],
                name,
                Some(&signature_text),
            );
        }
        if matches!(name, "Sum" | "Avg" | "Join" | "Action") {
            let before = self.diagnostics.len();
            let expected = match name {
                "Join" => SCALAR,
                "Action" => ACTION,
                _ => NUMBER | ABSENT,
            };
            if let Type::List(element) = &args[0] {
                self.require(
                    element,
                    expected,
                    &node.children[0],
                    name,
                    Some(&signature_text),
                );
            }
            if self.diagnostics.len() == before
                && matches!(node.children[0].kind, Kind::Expression(E::List))
            {
                for element in &node.children[0].children {
                    if literal_mask(element).is_some_and(|mask| mask & expected == 0) {
                        self.report(Code::ArgType, element, name, Some(&signature_text));
                    }
                }
            }
        }
        if matches!(name, "Min" | "Max") {
            let before = self.diagnostics.len();
            if args.len() == 1 {
                if let Type::List(element) = &args[0] {
                    self.require(
                        element,
                        NUMBER | STRING | ABSENT,
                        &node.children[0],
                        name,
                        Some(&signature_text),
                    );
                } else {
                    self.require(
                        &args[0],
                        LIST | ABSENT,
                        &node.children[0],
                        name,
                        Some(&signature_text),
                    );
                }
            } else {
                for (arg, child) in args.iter().zip(&node.children) {
                    self.require(
                        arg,
                        NUMBER | STRING | ABSENT,
                        child,
                        name,
                        Some(&signature_text),
                    );
                }
            }
            if args.len() > 1 {
                let common = args
                    .iter()
                    .filter(|arg| arg.mask() & ABSENT == 0)
                    .fold(NUMBER | STRING, |mask, arg| mask & arg.mask());
                if common == 0 && self.diagnostics.len() == before {
                    self.report(Code::ArgType, node, name, Some(&signature_text));
                }
            } else if matches!(node.children[0].kind, Kind::Expression(E::List)) {
                let common = node.children[0]
                    .children
                    .iter()
                    .filter_map(literal_mask)
                    .filter(|mask| mask & ABSENT == 0)
                    .fold(NUMBER | STRING, |common, mask| common & mask);
                if common == 0 && self.diagnostics.len() == before {
                    self.report(
                        Code::ArgType,
                        &node.children[0],
                        name,
                        Some(&signature_text),
                    );
                }
            }
        }
        match name {
            "Count" | "Sum" | "Round" | "Floor" | "Ceil" | "Abs" | "Sqrt" | "Clamp" | "Div"
            | "Len" | "ParseDate" => Type::Number,
            "Avg" | "Number" => Type::union([Type::Number, Type::Null]),
            "Min" | "Max" => {
                let candidates: Vec<_> = if args.len() == 1 {
                    match &args[0] {
                        Type::List(element) => vec![*element.clone()],
                        ty => vec![ty.clone()],
                    }
                } else {
                    args.clone()
                };
                let mask = candidates.iter().fold(0, |mask, ty| mask | ty.mask());
                let mut results = vec![Type::Null];
                if mask & NUMBER != 0 {
                    results.push(Type::Number);
                }
                if mask & STRING != 0 {
                    results.push(Type::String);
                }
                Type::union(results)
            }
            "First" | "Last" => args[0].element(),
            "Sort" | "Filter" => match &args[0] {
                Type::List(_) => args[0].clone(),
                _ => Type::List(Box::new(Type::Unknown)),
            },
            "Each" => Type::List(Box::new(args[2].clone())),
            "Range" => Type::List(Box::new(Type::Number)),
            "Keys" | "Split" | "Lines" => Type::List(Box::new(Type::String)),
            "Values" => Type::List(Box::new(match &args[0] {
                Type::Record(fields) => Type::union(fields.iter().map(|(_, ty)| ty.clone())),
                _ => Type::Unknown,
            })),
            "Slice" if matches!(args[0], Type::Null | Type::Missing) => Type::Missing,
            "Slice" => args[0].clone(),
            "Contains" | "IsError" | "StartsWith" | "EndsWith" => Type::Boolean,
            "Parse" => Type::Unknown,
            "Query" | "Watch" | "PagedList" => envelope(name),
            "Action" | "Set" | "Reset" | "Refresh" | "LoadMore" | "Compose" | "Open" => {
                Type::Action
            }
            _ => Type::String,
        }
    }
}

fn envelope(name: &str) -> Type {
    let mut fields: Vec<(String, Type)> = vec![
        ("status".into(), Type::String),
        ("error".into(), Type::union([Type::String, Type::Null])),
        ("stale".into(), Type::Boolean),
    ];
    match name {
        "Query" => fields.push(("value".into(), Type::Unknown)),
        "Watch" => fields.extend([
            ("value".into(), Type::List(Box::new(Type::Unknown))),
            ("dropped".into(), Type::Number),
        ]),
        _ => fields.extend([
            ("items".into(), Type::List(Box::new(Type::Unknown))),
            ("done".into(), Type::Boolean),
            ("total".into(), Type::union([Type::Number, Type::Null])),
            ("loadingMore".into(), Type::Boolean),
            ("capped".into(), Type::Boolean),
        ]),
    }
    Type::Envelope(
        match name {
            "Query" => SiteKind::Query,
            "Watch" => SiteKind::Watch,
            _ => SiteKind::PagedList,
        },
        fields,
    )
}

fn literal_mask(node: &Node) -> Option<u16> {
    match &node.kind {
        Kind::Expression(E::Number(_)) => Some(NUMBER),
        Kind::Expression(E::String(_)) => Some(STRING),
        Kind::Expression(E::Boolean(_)) => Some(BOOLEAN),
        Kind::Expression(E::Null) => Some(ABSENT),
        Kind::Expression(E::List) => Some(LIST),
        Kind::Expression(E::Record(_)) => Some(RECORD),
        Kind::Tag { .. } => Some(COMPONENT),
        Kind::Reference { .. } => Some(SNAPSHOT),
        Kind::Handle { .. } => Some(HANDLE),
        _ => None,
    }
}

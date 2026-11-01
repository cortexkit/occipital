use std::fmt;

use serde::{Deserialize, Serialize};

use super::Severity;

macro_rules! codes {
    ($($variant:ident => ($code:literal, $severity:ident, $message:literal)),+ $(,)?) => {
        /// Stable diagnostic identities from §12.4, independent of message wording.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub enum DiagnosticCode {
            $(#[doc = concat!("`", $code, "`: ", $message)] #[serde(rename = $code)] $variant),+
        }

        impl DiagnosticCode {
            /// Every code, in §12.4 order.
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            /// The code as written in diagnostics, such as `E_SYNTAX`.
            pub const fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $code),+ }
            }

            /// The severity §12.4 gives the code.
            pub const fn severity(self) -> Severity {
                match self { $(Self::$variant => Severity::$severity),+ }
            }

            // Fixed templates cannot leak snapshot or answer contents (§12.1).
            pub(super) const fn message(self) -> &'static str {
                match self { $(Self::$variant => $message),+ }
            }
        }
    };
}

codes! {
    SourceTooLarge => ("E_SOURCE_TOO_LARGE", Error, "Program source exceeds 64 KiB."),
    Syntax => ("E_SYNTAX", Error, "The source does not match the grammar here."),
    UnterminatedString => ("E_UNTERMINATED_STRING", Error, "A string literal reaches the end of its line."),
    BadEscape => ("E_BAD_ESCAPE", Error, "An escape is unknown or does not name a Unicode scalar."),
    NumberLiteral => ("E_NUMBER_LITERAL", Error, "A number literal has leading zeros, too many fraction digits, or is out of range."),
    UnclosedTag => ("E_UNCLOSED_TAG", Error, "A tag never closes."),
    MismatchedClose => ("E_MISMATCHED_CLOSE", Error, "A closing tag does not match the open tag."),
    DuplicateProp => ("E_DUPLICATE_PROP", Error, "A prop is given twice, or content and children are both given."),
    DuplicateKey => ("E_DUPLICATE_KEY", Error, "A record literal contains a duplicate key."),
    Nonassoc => ("E_NONASSOC", Error, "Comparison and equality operators cannot be chained without parentheses."),
    MixedCoalesce => ("E_MIXED_COALESCE", Error, "Coalescing and logical operators cannot be mixed without parentheses."),
    DeclarationIncomplete => ("E_DECLARATION_INCOMPLETE", Error, "A declaration is incomplete at a blank line or the end of source."),
    Nesting => ("E_NESTING", Error, "Expression or tag nesting exceeds its limit."),
    LimitDeclarations => ("E_LIMIT_DECLARATIONS", Error, "The program has more than 256 declarations."),
    UnknownName => ("E_UNKNOWN_NAME", Error, "A name is not declared or available in this scope."),
    UnknownState => ("E_UNKNOWN_STATE", Error, "A state name is not declared."),
    DuplicateName => ("E_DUPLICATE_NAME", Error, "Two declarations use the same name."),
    ReservedName => ("E_RESERVED_NAME", Error, "A declaration or binder uses a reserved name."),
    HostValueUnavailable => ("E_HOST_VALUE_UNAVAILABLE", Error, "This profile does not supply the host value."),
    ReferenceUnavailable => ("E_REFERENCE_UNAVAILABLE", Error, "References are not available in a tool view template."),
    ReferenceLimit => ("E_REFERENCE_LIMIT", Error, "The program has more than 64 reference occurrences or handles."),
    Cycle => ("E_CYCLE", Error, "Named values depend on each other in a cycle."),
    UnknownFunction => ("E_UNKNOWN_FUNCTION", Error, "The called name is not a built-in function."),
    Arity => ("E_ARITY", Error, "The call has the wrong number of arguments."),
    ArgType => ("E_ARG_TYPE", Error, "An argument has a statically incompatible type."),
    LiteralRequired => ("E_LITERAL_REQUIRED", Error, "A literal value is required here."),
    EachBinder => ("E_EACH_BINDER", Error, "The Each binder name is invalid."),
    EachNesting => ("E_EACH_NESTING", Error, "Each is nested more than four levels deep."),
    UnknownComponent => ("E_UNKNOWN_COMPONENT", Error, "The catalog has no component with this name."),
    UnknownProp => ("E_UNKNOWN_PROP", Error, "The component has no prop with this name."),
    MissingProp => ("E_MISSING_PROP", Error, "A required prop is missing."),
    Enum => ("E_ENUM", Error, "A literal is not a member of the prop's enum."),
    PropType => ("E_PROP_TYPE", Error, "A prop value has an incompatible type."),
    Placement => ("E_PLACEMENT", Error, "A component or its content is not allowed in this placement."),
    UnslottedRead => ("E_UNSLOTTED_READ", Error, "A tool row reads the call's view or the time outside a Slot."),
    Binding => ("E_BINDING", Error, "A binding requires a state of the correct type."),
    ActionRequired => ("E_ACTION_REQUIRED", Error, "An activated control requires an action prop."),
    ActionPlacement => ("E_ACTION_PLACEMENT", Error, "An action value is only allowed in an action prop."),
    LimitSteps => ("E_LIMIT_STEPS", Error, "An action has more than 16 local steps."),
    SitePlacement => ("E_SITE_PLACEMENT", Error, "A site constructor must be the whole expression of an exported named value."),
    SiteLimit => ("E_SITE_LIMIT", Error, "The program has too many sites."),
    SiteArgs => ("E_SITE_ARGS", Error, "Site arguments or defaults are not data, exceed 8 KiB, or do not match the schema."),
    EnvelopeMember => ("E_ENVELOPE_MEMBER", Error, "The site envelope has no such member."),
    RefreshRange => ("E_REFRESH_RANGE", Error, "A refresh interval must be between 2 and 3600 seconds."),
    UnknownOperation => ("E_UNKNOWN_OPERATION", Error, "The operation is not in the producer's registry."),
    OperationKind => ("E_OPERATION_KIND", Error, "The operation has the wrong query, watch or page kind."),
    WriteNotAllowed => ("E_WRITE_NOT_ALLOWED", Error, "Write constructors and steps are not allowed in language version 1."),
    ReplyForbidden => ("E_REPLY_FORBIDDEN", Error, "This tag or prop value is not allowed in replies."),
    LimitTags => ("E_LIMIT_TAGS", Error, "The reply has more than 64 tags."),
    UnsupportedVersion => ("E_UNSUPPORTED_VERSION", Error, "The program requires an unsupported language version."),
    Unused => ("W_UNUSED", Warning, "A named value or state is unused."),
    Encoding => ("W_ENCODING", Warning, "Invalid UTF-8 in the source was replaced."),
    Type => ("E_TYPE", Error, "An operand or argument has the wrong kind."),
    Arg => ("E_ARG", Error, "An argument has the right kind but an unusable value."),
    DivZero => ("E_DIV_ZERO", Error, "Division or remainder by zero is not defined."),
    Overflow => ("E_OVERFLOW", Error, "A number is out of range."),
    Domain => ("E_DOMAIN", Error, "The operation is not defined for this numeric argument."),
    NumberParse => ("E_NUMBER_PARSE", Error, "Number could not read the text as a number."),
    DateParse => ("E_DATE_PARSE", Error, "ParseDate could not read the text as a date."),
    Scheme => ("E_SCHEME", Error, "The address is invalid, is not HTTP or HTTPS, or exceeds 2048 scalars."),
    StateType => ("E_STATE_TYPE", Error, "A state assignment has the wrong type or exceeds the state's size limit."),
    Fuel => ("E_FUEL", Error, "Evaluation exhausted its fuel account."),
    LimitString => ("E_LIMIT_STRING", Error, "A string exceeds its scalar limit."),
    LimitList => ("E_LIMIT_LIST", Error, "A list exceeds its element limit."),
    LimitRecord => ("E_LIMIT_RECORD", Error, "A record exceeds its field limit."),
    LimitDepth => ("E_LIMIT_DEPTH", Error, "A value exceeds its nesting depth limit."),
    LimitValue => ("E_LIMIT_VALUE", Error, "A value exceeds its logical size limit."),
    LimitAlloc => ("E_LIMIT_ALLOC", Error, "Retained allocations exceed their account limit."),
    LimitNodes => ("E_LIMIT_NODES", Error, "The tree exceeds its node limit."),
    LimitText => ("E_LIMIT_TEXT", Error, "The tree exceeds its output text limit."),
    LimitLayout => ("E_LIMIT_LAYOUT", Error, "Layout work exceeds its page-visit reservation."),
    DataEncoding => ("E_DATA_ENCODING", Error, "Data is not valid UTF-8."),
    JsonSyntax => ("E_JSON_SYNTAX", Error, "Data does not match the JSON grammar."),
    JsonDuplicateKey => ("E_JSON_DUPLICATE_KEY", Error, "A JSON object contains a duplicate key."),
    CsvSyntax => ("E_CSV_SYNTAX", Error, "Data does not match the CSV grammar."),
    DataLimit => ("E_DATA_LIMIT", Error, "Data exceeds a parsing cap."),
    NumberRounded => ("W_NUMBER_ROUNDED", Warning, "A data number was rounded to 18 fraction digits."),
    NumberRange => ("W_NUMBER_RANGE", Warning, "A data number is out of range."),
    JsonSurrogate => ("W_JSON_SURROGATE", Warning, "An unpaired JSON surrogate was replaced."),
    CellType => ("W_CELL_TYPE", Warning, "A cell could not be converted to the requested type."),
    CsvHeaderRenamed => ("W_CSV_HEADER_RENAMED", Warning, "An empty or duplicate CSV header was renamed."),
    CsvShortRow => ("W_CSV_SHORT_ROW", Warning, "A CSV record has missing cells."),
    CsvLongRow => ("W_CSV_LONG_ROW", Warning, "A CSV record has extra cells."),
    CsvUnknownColumn => ("W_CSV_UNKNOWN_COLUMN", Warning, "A requested column is not in the CSV header."),
    RenderLimit => ("E_RENDER_LIMIT", Error, "Component data exceeds a renderer cap; nothing partial is drawn."),
}

impl fmt::Display for DiagnosticCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

import OccipitalCore

/// A text style layout measures in. The raw value is the ordinal the C
/// library passes to the metrics callbacks.
public enum TextStyle: Int32, Sendable, CaseIterable {
    case body = 0
    case subheadline = 1
    case footnote = 2
    case caption1 = 3
    case headline = 4
    case title2 = 5
    case title3 = 6
    case mono = 7
    case monoFootnote = 8
}

/// The platform text measurements layout needs, in points.
///
/// Every method must return a finite, non-negative value; anything else makes
/// the measuring call throw `OccipitalError.measureError`. The library rounds
/// each value up to 10⁻⁶ pt, so measured text is never shorter than drawn
/// text. The methods are called synchronously, on the calling thread, while a
/// measuring call runs, and must never call back into OccipitalKit.
public protocol TextMetrics {
    /// The height of one line in `style`.
    func l(style: TextStyle) -> Double

    /// The height of `text` in `style` wrapped to `width`, cut after
    /// `maxLines` lines; `maxLines` is `nil` for no limit.
    func t(text: String, style: TextStyle, width: Double, maxLines: Int?) -> Double

    /// The width of `text` in `style` set on one line.
    func tw(text: String, style: TextStyle) -> Double
}

/// Holds the metrics for the duration of one measuring call. The C callbacks
/// get it back through the table's `ctx` pointer.
private final class MetricsContext {
    let metrics: any TextMetrics

    init(_ metrics: any TextMetrics) {
        self.metrics = metrics
    }

    static func from(_ ctx: UnsafeMutableRawPointer?) -> MetricsContext? {
        guard let ctx else { return nil }
        return Unmanaged<MetricsContext>.fromOpaque(ctx).takeUnretainedValue()
    }
}

/// Decodes callback text: `len` bytes of UTF-8 that are not NUL-terminated.
private func callbackText(_ text: UnsafePointer<CChar>?, _ len: Int) -> String {
    String(decoding: UnsafeRawBufferPointer(start: text, count: len), as: UTF8.self)
}

/// Runs `body` with an `OccMetrics` callback table that forwards to `metrics`.
///
/// The table and the context it points to are valid only inside `body`; the
/// C library uses them only during the call made there. A missing context or
/// an unknown style ordinal returns NaN, which the library reports as
/// `OCC_MEASURE_ERROR` instead of measuring with a made-up value.
func withMetricsTable<R>(
    _ metrics: some TextMetrics,
    _ body: (UnsafePointer<OccMetrics>) -> R
) -> R {
    let context = MetricsContext(metrics)
    return withExtendedLifetime(context) {
        let table = OccMetrics(
            ctx: Unmanaged.passUnretained(context).toOpaque(),
            l: { ctx, style in
                guard let context = MetricsContext.from(ctx),
                    let style = TextStyle(rawValue: style)
                else { return .nan }
                return context.metrics.l(style: style)
            },
            t: { ctx, text, len, style, width, maxLines in
                guard let context = MetricsContext.from(ctx),
                    let style = TextStyle(rawValue: style)
                else { return .nan }
                return context.metrics.t(
                    text: callbackText(text, len),
                    style: style,
                    width: width,
                    // The library passes -1 for "no limit".
                    maxLines: maxLines < 0 ? nil : Int(maxLines)
                )
            },
            tw: { ctx, text, len, style in
                guard let context = MetricsContext.from(ctx),
                    let style = TextStyle(rawValue: style)
                else { return .nan }
                return context.metrics.tw(text: callbackText(text, len), style: style)
            }
        )
        return withUnsafePointer(to: table) { body($0) }
    }
}

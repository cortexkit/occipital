import OccipitalCore

/// A shared account that rich UI reserves work from: one chat account per
/// chat window, one app account per app process.
///
/// Accounts are thread-safe in the C library, so an `OccipitalAccount` may be
/// shared freely. A reply session keeps its own reference to the accounts it
/// was created with, so an account may be released while its sessions live.
public final class OccipitalAccount: @unchecked Sendable {
    // @unchecked: the handle never changes after init, and the C library
    // documents account handles as safe to use from concurrent calls.
    let handle: OpaquePointer

    private init(scope: Int32) throws(OccipitalError) {
        var out: OpaquePointer? = nil
        let status = occ_account_new(scope, &out)
        guard status == OCC_OK else {
            throw OccipitalError(status: status, output: nil)
        }
        guard let out else { throw .internal }
        handle = out
    }

    deinit {
        occ_account_free(handle)
    }

    /// A fresh chat account, shared by every reply in one chat window.
    public static func chat() throws(OccipitalError) -> OccipitalAccount {
        try OccipitalAccount(scope: OCC_SCOPE_CHAT)
    }

    /// A fresh app account, shared by all rich UI in the app process.
    public static func app() throws(OccipitalError) -> OccipitalAccount {
        try OccipitalAccount(scope: OCC_SCOPE_APP)
    }
}

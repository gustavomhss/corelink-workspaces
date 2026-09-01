//! The Win32 sequence. See the crate-level docs for why it lives alone.

/// Restrict `path` on Windows to a PROTECTED (inheritance-disabled) DACL that
/// grants FULL CONTROL to ONLY the current process user, dropping inherited /
/// Everyone access. This is the Windows analogue of the unix `0600`/`0700`
/// hardening, which is a no-op on Windows — without it the plaintext-PAT
/// `~/.clw/config.toml` inherits the (often group/Users-readable) parent ACL.
///
/// Cold-review map of the Win32 sequence (each step documented at its call):
///
/// 1. `OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, ..)` — token handle.
/// 2. `GetTokenInformation(token, TokenUser, ..)` (size-then-buffer) — our SID.
/// 3. `InitializeAcl` + `AddAccessAllowedAce(.., GENERIC_ALL, sid)` — a one-ACE
///    DACL over a correctly sized buffer.
/// 4. `SetNamedSecurityInfoW(.., DACL_SECURITY_INFORMATION |
///    PROTECTED_DACL_SECURITY_INFORMATION, ..)` — replace the object's DACL and
///    disable inheritance.
///
/// All handles/buffers are released before returning; any non-`ERROR_SUCCESS`
/// status is mapped to an `io::Error`.
pub fn restrict_to_owner(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt as _;

    // GENERIC_ALL grants the owner full control over the object; ERROR_SUCCESS is
    // the success code SetNamedSecurityInfoW returns directly (not via LastError).
    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, GENERIC_ALL, HANDLE};
    use windows_sys::Win32::Security::Authorization::{SetNamedSecurityInfoW, SE_FILE_OBJECT};
    use windows_sys::Win32::Security::{
        AddAccessAllowedAce, GetLengthSid, GetTokenInformation, InitializeAcl, TokenUser,
        ACCESS_ALLOWED_ACE, ACL, ACL_REVISION, DACL_SECURITY_INFORMATION,
        PROTECTED_DACL_SECURITY_INFORMATION, PSID, TOKEN_QUERY, TOKEN_USER,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    // Helper: snapshot the last OS error into an io::Error (read by
    // `io::Error::last_os_error`, which calls GetLastError under the hood).
    let last_err = || std::io::Error::last_os_error();

    // SAFETY: every call below is a documented Win32 API used per its contract:
    // handles/buffers are sized via the prescribed two-call pattern, the ACL
    // buffer is sized exactly, and all owned resources (token handle, SID buffer)
    // are released on every return path before the function exits.
    //
    // NECESSARY: there is no safe Rust API for replacing a file's Windows DACL —
    // the hardening below must call the Win32 security APIs through `windows-sys`
    // FFI, which is `unsafe` by definition. Without it the plaintext-PAT config
    // inherits a group-readable ACL (see the doc comment above).
    //
    // Why this `allow` exists and why there is exactly one: the workspace lint is
    // `forbid(unsafe_code)`, which cannot be locally re-allowed, so THIS CRATE —
    // and only this crate — restates it at `deny` in its Cargo.toml so that this
    // one block can be allowed here under review. What the unsafe-posture guard in
    // `scripts/gates.sh` asserts is that this crate is UNCHANGED against its pinned
    // fixture — not that the count is one. Nothing counts attributes any more: five
    // generations of a guard that did were each beaten by a spelling, so the question
    // became "did a human read the diff of this crate", which a content hash answers
    // exactly. That every OTHER crate is free of them is the compiler's answer, not a
    // scan's: `forbid` rejects a permission attribute however it is written.
    //
    // The per-call safety argument is the `SAFETY:` note above; it is deliberately
    // not restated here, because a summary that generalises ("buffers are sized by
    // the two-call pattern") is less true than the original, which distinguishes the
    // two-call-sized TOKEN_USER buffer from the arithmetic-sized ACL buffer.
    #[allow(unsafe_code)]
    unsafe {
        // --- 1. Open the current process token for query --------------------
        // OpenProcessToken on the pseudo-handle from GetCurrentProcess() yields a
        // token we can interrogate for the running user's SID. TOKEN_QUERY is the
        // only access right needed.
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(last_err());
        }

        // --- 2a. Size the TokenUser buffer ----------------------------------
        // First GetTokenInformation call: pass a NULL buffer / 0 length to learn
        // the required size (it "fails" with ERROR_INSUFFICIENT_BUFFER, expected).
        let mut needed: u32 = 0;
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut needed);
        if needed == 0 {
            let e = last_err();
            CloseHandle(token);
            return Err(e);
        }

        // --- 2b. Fetch the TOKEN_USER (which embeds the user's SID) ---------
        // Second call: an OVER-ALIGNED buffer backs the variable-length TOKEN_USER;
        // the SID it points to lives INSIDE this buffer, so `buf` must outlive every
        // use of `sid` below.
        //
        // FIX 5 (soundness): TOKEN_USER embeds a SID_AND_ATTRIBUTES whose `Sid`
        // field is a POINTER (alignment 8), so the backing store must be 8-aligned.
        // A plain `Vec<u8>` is align-1; casting it to `*const TOKEN_USER` and reading
        // through it is UNDEFINED BEHAVIOR (misaligned read). Backing the buffer with
        // a `Vec<u64>` (align 8) sized to ceil(needed/8) gives a correctly aligned
        // allocation while keeping the exact byte size the API asked for.
        let mut buf: Vec<u64> = vec![0u64; needed.div_ceil(8) as usize];
        if GetTokenInformation(
            token,
            TokenUser,
            buf.as_mut_ptr().cast(),
            needed,
            &mut needed,
        ) == 0
        {
            let e = last_err();
            CloseHandle(token);
            return Err(e);
        }
        // The token handle is no longer needed once the SID is copied into `buf`.
        CloseHandle(token);

        // TOKEN_USER { User: SID_AND_ATTRIBUTES { Sid: PSID, .. } }.
        let token_user = &*(buf.as_ptr() as *const TOKEN_USER);
        let sid: PSID = token_user.User.Sid;
        if sid.is_null() {
            return Err(std::io::Error::other("TokenUser returned a null SID"));
        }

        // --- 3. Build a one-ACE DACL granting GENERIC_ALL to our SID --------
        // ACL byte size = sizeof(ACL) header
        //               + sizeof(ACCESS_ALLOWED_ACE) (its trailing SidStart DWORD
        //                 is a placeholder for the SID's first dword)
        //               - sizeof(DWORD) (remove that placeholder)
        //               + GetLengthSid(sid) (the actual SID bytes).
        let sid_len = GetLengthSid(sid);
        if sid_len == 0 {
            return Err(last_err());
        }
        let acl_size = std::mem::size_of::<ACL>() + std::mem::size_of::<ACCESS_ALLOWED_ACE>()
            - std::mem::size_of::<u32>()
            + sid_len as usize;
        // FIX 5 (soundness): ACL and ACCESS_ALLOWED_ACE contain u32 fields (align 4),
        // so writing/reading through this buffer as `*mut ACL` requires ≥4-byte
        // alignment. A plain `Vec<u8>` is align-1, so the cast + InitializeAcl write
        // would be UNDEFINED BEHAVIOR. Back it with a `Vec<u64>` (align 8, which
        // covers the align-4 need) sized to ceil(acl_size/8) — same byte size, just
        // an over-aligned allocation.
        let mut acl_buf: Vec<u64> = vec![0u64; acl_size.div_ceil(8)];
        let acl = acl_buf.as_mut_ptr() as *mut ACL;

        // InitializeAcl lays down the ACL header in our buffer.
        if InitializeAcl(acl, acl_size as u32, ACL_REVISION) == 0 {
            return Err(last_err());
        }
        // AddAccessAllowedAce appends a single allow-ACE: GENERIC_ALL for our SID.
        if AddAccessAllowedAce(acl, ACL_REVISION, GENERIC_ALL, sid) == 0 {
            return Err(last_err());
        }

        // --- 4. Apply the DACL to the named file object ---------------------
        // Wide, NUL-terminated path for the *W API (OsStr::encode_wide + 0).
        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        // SetNamedSecurityInfoW: pobjectname is PCWSTR (*const u16). securityinfo =
        // DACL_SECURITY_INFORMATION ("I'm setting the DACL") |
        // PROTECTED_DACL_SECURITY_INFORMATION (mark it PROTECTED so inherited ACEs
        // are dropped — the explicit owner-only ACE becomes the entire effective
        // DACL). psidowner/psidgroup (PSID = *mut) and psacl (*const ACL) are NULL,
        // leaving owner/group/SACL untouched; pdacl points at our owner-only ACL.
        let status = SetNamedSecurityInfoW(
            wide.as_ptr(), // pObjectName — PCWSTR
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(), // psidOwner — unchanged
            std::ptr::null_mut(), // psidGroup — unchanged
            acl.cast_const(),     // pDacl     — our owner-only DACL (*const ACL)
            std::ptr::null(),     // pSacl     — unchanged (*const ACL)
        );

        // SetNamedSecurityInfoW returns a WIN32_ERROR code DIRECTLY (NOT via
        // GetLastError); map a non-ERROR_SUCCESS status to an io::Error.
        if status != ERROR_SUCCESS {
            return Err(std::io::Error::from_raw_os_error(status as i32));
        }

        Ok(())
    }
}

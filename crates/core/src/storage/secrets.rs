// DPAPI保护登录凭据。
use crate::deps::{CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, LocalFree, Zeroize};
use crate::domain::error::{AppError, StorageError};

/// 加密或解密有界凭据并清理Windows分配的明文。
pub(crate) fn prtct(bytes: &[u8], decrypt: bool) -> Result<Vec<u8>, AppError> {
    if bytes.len() > 128 * 1024 { return Err(StorageError::Corrupt.into()); }
    let input = CRYPT_INTEGER_BLOB { cbData: bytes.len() as u32, pbData: bytes.as_ptr() as *mut u8 };
    let mut output: CRYPT_INTEGER_BLOB = unsafe { std::mem::zeroed() };
    let success = unsafe {
        if decrypt { CryptUnprotectData(&input, std::ptr::null_mut(), std::ptr::null(), std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut output) }
        else { CryptProtectData(&input, std::ptr::null(), std::ptr::null(), std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut output) }
    };
    if success == 0 { return Err(StorageError::Corrupt.into()); }
    if output.cbData == 0 || output.pbData.is_null() {
        if !output.pbData.is_null() { unsafe { LocalFree(output.pbData as *mut _); } }
        return Ok(Vec::new());
    }
    let result = unsafe {
        let data = std::slice::from_raw_parts_mut(output.pbData, output.cbData as usize);
        let copy = if data.len() <= 128 * 1024 { Ok(data.to_vec()) } else { Err(StorageError::Corrupt.into()) };
        data.zeroize();
        LocalFree(output.pbData as *mut _);
        copy
    };
    result
}

#[cfg(test)]
mod tests {
    /// 验证Windows用户加密往返及密文篡改拒绝。
    #[test]
    fn tst_dpapi() {
        let plain = b"KoDog synthetic test credential";
        let mut encrypted = super::prtct(plain, false).unwrap();
        assert_ne!(encrypted.as_slice(), plain);
        assert_eq!(super::prtct(&encrypted, true).unwrap(), plain);
        *encrypted.last_mut().unwrap() ^= 1;
        assert!(super::prtct(&encrypted, true).is_err());
        assert!(super::prtct(&[], true).is_err());
    }
}

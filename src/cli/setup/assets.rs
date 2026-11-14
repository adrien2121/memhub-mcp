//! Pinned download URLs and SHA-256 hashes for `setup`.

/// Where a pinned file comes from and how to check it.
pub(super) struct Asset {
    /// Download URL, pinned to a fixed revision or release.
    pub url: &'static str,
    /// For a `.tgz`/`.zip`/`.whl`: SHA-256 of the download and the path inside it.
    pub archive: Option<(&'static str, &'static str)>,
    /// SHA-256 of the installed file.
    pub sha256: &'static str,
}

/// bge-small-en-v1.5 file URL at the revision recorded in the default `model_id`.
macro_rules! hf {
    ($file:literal) => {
        concat!(
            "https://huggingface.co/Xenova/bge-small-en-v1.5/resolve/ea104dacec62c0de699686887e3f920caeb4f3e3/",
            $file
        )
    };
}

pub(super) const MODEL: Asset = Asset {
    url: hf!("onnx/model_quantized.onnx"),
    archive: None,
    sha256: "6c9c6101a956d62dfb5e7190c538226c0c5bb9cb27b651234b6df063ee7dbfe4",
};

pub(super) const TOKENIZER: Asset = Asset {
    url: hf!("tokenizer.json"),
    archive: None,
    sha256: "d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66",
};

/// onnxruntime 1.31.0 for this platform (macOS: the official PyPI wheel's dylib, without the
/// GitHub build's debug symbols); `None` where no official build exists (e.g. macOS x86_64).
pub(super) const ORT: Option<Asset> = if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
    Some(Asset {
        url: "https://files.pythonhosted.org/packages/e0/2b/117f94d73a3bac4276c285c47e384e1b3ea67b191aa4c7592df9d3f4a136/onnxruntime-1.31.0-cp313-cp313-macosx_14_0_arm64.whl",
        archive: Some((
            "0ba02a44acb6203040354d9a1f160e3f37a43feac7bb05caa3e0ea545efed505",
            "onnxruntime/capi/libonnxruntime.1.31.0.dylib",
        )),
        sha256: "9d2945b4de2ca594608f33b99498c8a81e6696e75da7ae9c67bc9ad3cc36c0d6",
    })
} else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
    Some(Asset {
        url: "https://github.com/microsoft/onnxruntime/releases/download/v1.31.0/onnxruntime-linux-x64-1.31.0.tgz",
        archive: Some((
            "cc5c72baf5ae5c8238a6841f0897227be2d02826b9cf98eaf02fdefaeeb03a57",
            "onnxruntime-linux-x64-1.31.0/lib/libonnxruntime.so.1.31.0",
        )),
        sha256: "469849977cb60744ea160927e1a3a0643fd4c55c0aff743251c6d293402e334e",
    })
} else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
    Some(Asset {
        url: "https://github.com/microsoft/onnxruntime/releases/download/v1.31.0/onnxruntime-linux-aarch64-1.31.0.tgz",
        archive: Some((
            "c5b8b3cca31f3d643a3b313b8f42e2d0f16a4fcefd02cda343d97bd5306afaab",
            "onnxruntime-linux-aarch64-1.31.0/lib/libonnxruntime.so.1.31.0",
        )),
        sha256: "595c7f7a6775f5c7b3d567c3ed94c04f722092575abcc092560f5208a54daf99",
    })
} else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
    Some(Asset {
        url: "https://github.com/microsoft/onnxruntime/releases/download/v1.31.0/onnxruntime-win-x64-1.31.0.zip",
        archive: Some((
            "3958d8a44984160692c1b4ffd0de3cb48f8c39f9d34bb31e433db06e30b82195",
            "onnxruntime-win-x64-1.31.0/lib/onnxruntime.dll",
        )),
        sha256: "0c42dc4e278f0968fdf9ebf1b1e54a74c657a9942e70803e453c91993451cf4c",
    })
} else {
    None
};

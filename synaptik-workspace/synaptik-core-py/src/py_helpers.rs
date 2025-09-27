use blake3;
use chrono::Utc;
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyBool, PyDict, PyFloat, PyList, PyLong};
use serde_json::{json, Value};

use syn_core::utils::pons::{ObjectMetadata as PonsMetadata, ObjectRef as PonsObjectRef};
use synaptik_core as syn_core;

pub fn pyerr<E: std::fmt::Display>(e: E) -> PyErr {
    pyo3::exceptions::PyRuntimeError::new_err(e.to_string())
}

// -------- name & path helpers --------

pub fn sanitize_name(name: &str) -> String {
    let mut s = name.to_lowercase();
    s.retain(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if s.is_empty() {
        s = "path".into();
    }
    s
}

// Mirror DAG's sanitize for path id filenames: replace non-alnum with '_', preserve case.
pub fn dag_path_id(name: &str) -> String {
    name.chars()
        .map(|c| c.is_ascii_alphanumeric().then_some(c).unwrap_or('_'))
        .collect()
}

pub fn gen_path_name(prefix: &str, seed: &str) -> String {
    let ts = Utc::now().format("%Y%m%d-%H%M%S");
    let sh = &blake3::hash(seed.as_bytes()).to_hex()[..8];
    format!("{}-{}-{}", prefix, ts, sh)
}

/// Bind JSON: shallow key binding of overlay into base (overlay wins on conflicts).
pub fn bind_json(base: Value, overlay: Value) -> Value {
    use serde_json::Value::*;
    match (base, overlay) {
        (Object(mut b), Object(o)) => {
            for (k, v) in o {
                b.insert(k, v);
            }
            Object(b)
        }
        (_, o) => o,
    }
}

pub fn path_exists_in_refs(path_name: &str) -> anyhow::Result<bool> {
    let rep = syn_core::commands::init::ensure_initialized_once()?;
    let norm = sanitize_name(path_name);
    let pid = dag_path_id(&norm);
    let p = rep
        .root
        .join("refs")
        .join("paths")
        .join(format!("{}.json", pid));
    Ok(p.exists())
}

// -------- JSON <-> Python conversion helpers --------

pub fn json_to_py(py: Python<'_>, v: &Value) -> PyObject {
    use serde_json::Value::*;
    match v {
        Null => py.None().into_py(py),
        Bool(b) => b.into_py(py),
        Number(n) => {
            if let Some(i) = n.as_i64() {
                i.into_py(py)
            } else if let Some(u) = n.as_u64() {
                (u as i128).into_py(py)
            } else if let Some(f) = n.as_f64() {
                f.into_py(py)
            } else {
                py.None().into_py(py)
            }
        }
        String(s) => s.into_py(py),
        Array(arr) => {
            let list = PyList::empty_bound(py);
            for item in arr {
                list.append(json_to_py(py, item)).ok();
            }
            list.into_any().into_py(py)
        }
        Object(map) => {
            let d = PyDict::new_bound(py);
            for (k, val) in map.iter() {
                let _ = d.set_item(k, json_to_py(py, val));
            }
            d.into_any().into_py(py)
        }
    }
}

pub fn json_array_to_py(py: Python<'_>, arr: &[Value]) -> PyObject {
    let list = PyList::empty_bound(py);
    for item in arr {
        let _ = list.append(json_to_py(py, item));
    }
    list.into_any().into_py(py)
}

pub fn py_to_json(any: &Bound<'_, PyAny>) -> Value {
    // Treat only a real Python bool (PyBool) as JSON Bool. Avoid generic truthiness coercion
    // (e.g., non-empty lists, custom objects with __bool__/__len__). This preserves type intent.
    if let Ok(bobj) = any.downcast::<PyBool>() {
        return Value::Bool(bobj.is_true());
    }
    if let Ok(i) = any.extract::<i64>() {
        return json!(i);
    }
    // Preserve integers first (signed/unsigned) to avoid precision loss for large ints.
    if let Ok(u) = any.extract::<u64>() {
        return Value::Number(serde_json::Number::from(u));
    }
    // Float / numeric-like handling:
    // Accept if it's a real PyFloat OR (not a PyLong) AND extract::<f64>() succeeds (covers numpy.float64, decimal.Decimal).
    let is_pylong = any.downcast::<PyLong>().is_ok();
    if let Ok(pyfloat) = any.downcast::<PyFloat>() {
        let f = pyfloat.value();
        if let Some(num) = serde_json::Number::from_f64(f) {
            return Value::Number(num);
        } else {
            return json!(f.to_string());
        }
    } else if !is_pylong {
        if let Ok(f) = any.extract::<f64>() {
            if let Some(num) = serde_json::Number::from_f64(f) {
                return Value::Number(num);
            } else {
                return json!(f.to_string());
            }
        }
    }
    if let Ok(s) = any.extract::<String>() {
        return json!(s);
    }
    if any.is_none() {
        return Value::Null;
    }
    if let Ok(dict) = any.downcast::<PyDict>() {
        let mut m = serde_json::Map::new();
        for (k, v) in dict.iter() {
            // Try direct extraction to Rust String first.
            let key_str = match k.extract::<String>() {
                Ok(s) => s,
                Err(_) => {
                    // Fallback to Python str() representation; if that fails, skip key.
                    match k.str() {
                        Ok(pystr) => pystr.to_string(),
                        Err(_) => continue,
                    }
                }
            };
            if key_str.is_empty() {
                continue;
            } // Avoid inserting empty key from non-string objects.
              // Do not overwrite existing entries silently; keep the first occurrence.
            if !m.contains_key(&key_str) {
                m.insert(key_str, py_to_json(&v));
            }
        }
        return Value::Object(m);
    }
    if let Ok(list) = any.downcast::<PyList>() {
        let mut a = Vec::with_capacity(list.len());
        for v in list.iter() {
            a.push(py_to_json(&v));
        }
        return Value::Array(a);
    }
    if let Ok(s) = any.str() {
        return json!(s.to_string());
    }
    Value::Null
}

pub fn object_ref_to_py(py: Python<'_>, r: &PonsObjectRef) -> PyObject {
    let d = PyDict::new_bound(py);
    let _ = d.set_item("pons", &r.pons);
    let _ = d.set_item("key", &r.key);
    let _ = d.set_item("version", r.version.clone());
    let _ = d.set_item("etag", r.etag.clone());
    let _ = d.set_item("size_bytes", r.size_bytes);
    d.into_any().into_py(py)
}

pub fn metadata_to_py(py: Python<'_>, meta: &PonsMetadata) -> PyObject {
    let d = PyDict::new_bound(py);
    let _ = d.set_item("media_type", meta.media_type.clone());
    match &meta.extra {
        Some(v) => {
            let _ = d.set_item("extra", json_to_py(py, v));
        }
        None => {
            let _ = d.set_item("extra", py.None());
        }
    }
    d.into_any().into_py(py)
}

//! Native part of the `lsdocs_rst` Python package.
//!
//! The Python-facing API lives in `python/lsdocs_rst/__init__.py`; this
//! module provides what it wraps:
//!
//! - `NativeParser`: an `lsdocs_rst::Parser` with Python directives, roles
//!   and file loaders. Parsing runs with the GIL released.
//! - `pformat`, `to_xml`: writers taking a tree in its dict form.
//! - `make_id`, `default_settings`.
//!
//! Trees cross as dicts in the form of the `serde` feature of `lsdocs-rst`
//! (`{"kind", "attributes", "children", "span"}`, `{"text", "span"}`).
//!
//! A directive written in Python gets a context whose methods
//! (`parse_content`, ...) need the parser's `DirectiveContext`, which only
//! lives on the parsing thread. So the Python callable runs on a scoped
//! worker thread, and its context sends requests over a channel to the
//! parsing thread, which answers them until the callable returns. The
//! context releases the GIL while it waits, so directives nested in the
//! content of a Python directive can run Python too.

use std::io;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};

use lsdocs_rst::{
    Directive, DirectiveContext, DirectiveError, DirectiveSpec, Element, FileLoader, Node,
    OptionKind, OptionValue, Parser, Role, RoleContext, Settings, Span, XmlOptions,
};
use pyo3::exceptions::{
    PyFileNotFoundError, PyIsADirectoryError, PyOSError, PyPermissionError, PyRuntimeError,
    PyTypeError, PyValueError,
};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList, PyString, PyTuple};
use pythonize::{depythonize, pythonize};

/// Python exceptions raised by callbacks during a parse; the first is
/// raised again once the parse is done.
type Raised = Arc<Mutex<Vec<PyErr>>>;

fn keep(raised: &Raised, err: PyErr) {
    raised
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(err);
}

fn value_error(e: impl std::fmt::Display) -> PyErr {
    PyValueError::new_err(e.to_string())
}

fn to_py<'py, T: serde::Serialize>(py: Python<'py>, value: &T) -> PyResult<Bound<'py, PyAny>> {
    pythonize(py, value).map_err(value_error)
}

fn span_to_py(py: Python<'_>, span: Option<Span>) -> PyResult<Bound<'_, PyAny>> {
    to_py(py, &span)
}

/// An option value as docutils' conversion functions return it.
fn option_to_py<'py>(py: Python<'py>, value: &OptionValue) -> PyResult<Bound<'py, PyAny>> {
    Ok(match value {
        OptionValue::Flag => py.None().into_bound(py),
        OptionValue::Text(s) => PyString::new(py, s).into_any(),
        OptionValue::Int(n) => n.into_pyobject(py)?.into_any(),
        OptionValue::List(l) => PyList::new(py, l)?.into_any(),
        _ => py.None().into_bound(py),
    })
}

fn options_to_py<'py>(
    py: Python<'py>,
    options: &[(String, OptionValue)],
) -> PyResult<Bound<'py, PyDict>> {
    let dict = PyDict::new(py);
    for (name, value) in options {
        dict.set_item(name, option_to_py(py, value)?)?;
    }
    Ok(dict)
}

/// A node list returned by the Python wrapper: `("ok", [node dicts])` or
/// `("error", level, message)`.
fn outcome(result: &Bound<'_, PyAny>) -> PyResult<Result<Vec<Node>, DirectiveError>> {
    let tuple = result.cast::<PyTuple>()?;
    let tag: String = tuple.get_item(0)?.extract()?;
    if tag == "ok" {
        let nodes: Vec<Node> = depythonize(&tuple.get_item(1)?).map_err(value_error)?;
        Ok(Ok(nodes))
    } else {
        let level: i64 = tuple.get_item(1)?.extract()?;
        let message: String = tuple.get_item(2)?.extract()?;
        Ok(Err(DirectiveError { level, message }))
    }
}

// -- directives -----------------------------------------------------------------

/// What the context asks the parsing thread for.
enum Request {
    ParseContent(Element, Sender<Element>),
    ParseInline(String, Sender<(Vec<Node>, Vec<Element>)>),
    Message(i64, String, Sender<Element>),
}

/// A directive's context, as Python sees it. Use
/// `lsdocs_rst.DirectiveContext` rather than this class.
#[pyclass(module = "lsdocs_rst._native", frozen)]
struct NativeContext {
    name: String,
    arguments: Vec<String>,
    options: Vec<(String, OptionValue)>,
    content: Vec<String>,
    lineno: usize,
    block_text: String,
    span: Option<Span>,
    in_substitution: bool,
    /// Taken away when the directive returns, which ends the parsing
    /// thread's wait for requests.
    requests: Mutex<Option<Sender<Request>>>,
}

impl NativeContext {
    /// Sends a request and waits for its answer, with the GIL released.
    fn ask<T: Send>(
        &self,
        py: Python<'_>,
        request: impl FnOnce(Sender<T>) -> Request,
    ) -> PyResult<T> {
        let requests = self
            .requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let Some(requests) = requests else {
            return Err(PyRuntimeError::new_err(
                "the directive has returned; its context can no longer parse",
            ));
        };
        let (tx, rx) = channel();
        let request = request(tx);
        py.detach(move || {
            requests.send(request).ok()?;
            rx.recv().ok()
        })
        .ok_or_else(|| PyRuntimeError::new_err("the parser stopped answering"))
    }

    fn close(&self) {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
    }
}

#[pymethods]
impl NativeContext {
    #[getter]
    fn name(&self) -> &str {
        &self.name
    }

    #[getter]
    fn arguments(&self) -> Vec<String> {
        self.arguments.clone()
    }

    #[getter]
    fn options<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        options_to_py(py, &self.options)
    }

    #[getter]
    fn content(&self) -> Vec<String> {
        self.content.clone()
    }

    #[getter]
    fn lineno(&self) -> usize {
        self.lineno
    }

    #[getter]
    fn block_text(&self) -> &str {
        &self.block_text
    }

    #[getter]
    fn span<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        span_to_py(py, self.span)
    }

    #[getter]
    fn in_substitution(&self) -> bool {
        self.in_substitution
    }

    /// Parses the content into `container` (a dict), returned as a dict.
    fn parse_content<'py>(
        &self,
        py: Python<'py>,
        container: &Bound<'py, PyAny>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let container: Element = depythonize(container).map_err(value_error)?;
        let element = self.ask(py, |tx| Request::ParseContent(container, tx))?;
        to_py(py, &element)
    }

    /// Parses `text` as inline markup: `(nodes, messages)` as dicts.
    fn parse_inline<'py>(&self, py: Python<'py>, text: String) -> PyResult<Bound<'py, PyTuple>> {
        let (nodes, messages) = self.ask(py, |tx| Request::ParseInline(text, tx))?;
        PyTuple::new(py, [to_py(py, &nodes)?, to_py(py, &messages)?])
    }

    /// A system message for the directive's line, as a dict.
    fn message<'py>(
        &self,
        py: Python<'py>,
        level: i64,
        text: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let element = self.ask(py, |tx| Request::Message(level, text, tx))?;
        to_py(py, &element)
    }
}

/// A directive whose `run` is Python: the package's wrapper, called with
/// the context and returning an outcome tuple.
struct PyDirective {
    spec: DirectiveSpec,
    run: Py<PyAny>,
    raised: Raised,
}

impl Directive for PyDirective {
    fn spec(&self) -> DirectiveSpec {
        self.spec.clone()
    }

    fn run(&self, cx: &mut DirectiveContext<'_>) -> Result<Vec<Node>, DirectiveError> {
        let (tx, rx): (Sender<Request>, Receiver<Request>) = channel();
        let context = NativeContext {
            name: cx.name().to_owned(),
            arguments: cx.arguments().to_vec(),
            options: cx.options(),
            content: cx.content(),
            lineno: cx.line(),
            block_text: cx.block_text().to_owned(),
            span: cx.span(),
            in_substitution: cx.in_substitution(),
            requests: Mutex::new(Some(tx)),
        };
        let name = context.name.clone();
        std::thread::scope(|scope| {
            let worker = scope.spawn(|| {
                Python::attach(|py| -> PyResult<Result<Vec<Node>, DirectiveError>> {
                    let context = Py::new(py, context)?;
                    let result = self.run.call1(py, (context.clone_ref(py),));
                    context.get().close();
                    outcome(result?.bind(py))
                })
            });
            // Answer the context's requests until it is closed.
            for request in rx {
                match request {
                    Request::ParseContent(container, reply) => {
                        let _ = reply.send(cx.parse_content(container));
                    }
                    Request::ParseInline(text, reply) => {
                        let _ = reply.send(cx.parse_inline(&text));
                    }
                    Request::Message(level, text, reply) => {
                        let _ = reply.send(cx.message(level, &text));
                    }
                }
            }
            match worker.join() {
                Ok(Ok(result)) => result,
                Ok(Err(err)) => {
                    keep(&self.raised, err);
                    Err(DirectiveError::error(format!(
                        "Directive \"{name}\" raised an exception."
                    )))
                }
                Err(_) => Err(DirectiveError::error(format!(
                    "Directive \"{name}\" panicked."
                ))),
            }
        })
    }
}

/// An option kind by its docutils conversion function's name, or a list
/// of choices.
fn option_kind(value: &Bound<'_, PyAny>) -> PyResult<OptionKind> {
    if let Ok(choices) = value.extract::<Vec<String>>()
        && !value.is_instance_of::<PyString>()
    {
        return Ok(OptionKind::Choice(choices));
    }
    let name: String = value.extract()?;
    Ok(match name.as_str() {
        "flag" => OptionKind::Flag,
        "unchanged" => OptionKind::Text,
        "unchanged_required" => OptionKind::RequiredText,
        "path" => OptionKind::Path,
        "uri" => OptionKind::Uri,
        "int" => OptionKind::Int,
        "nonnegative_int" => OptionKind::NonNegativeInt,
        "percentage" => OptionKind::Percentage,
        "length_or_unitless" => OptionKind::Length,
        "length_or_percentage_or_unitless" => OptionKind::LengthOrPercentage,
        "class_option" => OptionKind::Class,
        other => {
            return Err(PyValueError::new_err(format!(
                "unknown option kind {other:?}"
            )));
        }
    })
}

/// A directive spec from its dict form.
fn directive_spec(spec: &Bound<'_, PyDict>) -> PyResult<DirectiveSpec> {
    let get = |name: &str| spec.get_item(name);
    let mut out = DirectiveSpec::default();
    if let Some(v) = get("required_arguments")? {
        out.required_arguments = v.extract()?;
    }
    if let Some(v) = get("optional_arguments")? {
        out.optional_arguments = v.extract()?;
    }
    if let Some(v) = get("final_argument_whitespace")? {
        out.final_argument_whitespace = v.is_truthy()?;
    }
    if let Some(v) = get("has_content")? {
        out.has_content = v.is_truthy()?;
    }
    if let Some(v) = get("options")? {
        for (name, kind) in v.cast::<PyDict>()?.iter() {
            out.options.push((name.extract()?, option_kind(&kind)?));
        }
    }
    Ok(out)
}

// -- roles and file loading ------------------------------------------------------

/// A role whose `run` is Python: called with the role's data as a dict,
/// returning an outcome tuple.
struct PyRole {
    run: Py<PyAny>,
    raised: Raised,
}

impl Role for PyRole {
    fn run(&self, cx: &mut RoleContext<'_>) -> Result<Vec<Node>, DirectiveError> {
        Python::attach(|py| {
            let call = || -> PyResult<Result<Vec<Node>, DirectiveError>> {
                let data = PyDict::new(py);
                data.set_item("name", cx.name())?;
                data.set_item("text", cx.text())?;
                data.set_item("raw_text", cx.raw_text())?;
                data.set_item("rawsource", cx.rawsource())?;
                data.set_item("options", options_to_py(py, &cx.options())?)?;
                data.set_item("lineno", cx.line())?;
                data.set_item("span", span_to_py(py, cx.span())?)?;
                outcome(self.run.call1(py, (data,))?.bind(py))
            };
            call().unwrap_or_else(|err| {
                keep(&self.raised, err);
                Err(DirectiveError::error(format!(
                    "Role \"{}\" raised an exception.",
                    cx.name()
                )))
            })
        })
    }
}

/// File and URL access through Python callables returning bytes. An
/// `OSError` is a failed read, reported in the document; any other
/// exception is raised again after the parse.
struct PyLoader {
    files: Option<Py<PyAny>>,
    urls: Option<Py<PyAny>>,
    raised: Raised,
}

impl PyLoader {
    fn read(&self, func: Option<&Py<PyAny>>, arg: &str) -> io::Result<Vec<u8>> {
        let Some(func) = func else {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "URL access is not supported",
            ));
        };
        Python::attach(|py| {
            let result = func.call1(py, (arg,)).and_then(|r| {
                let r = r.bind(py);
                if let Ok(s) = r.cast::<PyString>() {
                    Ok(s.to_str()?.as_bytes().to_vec())
                } else {
                    Ok(r.cast::<PyBytes>()?.as_bytes().to_vec())
                }
            });
            result.map_err(|err| {
                let kind = if err.is_instance_of::<PyFileNotFoundError>(py) {
                    io::ErrorKind::NotFound
                } else if err.is_instance_of::<PyPermissionError>(py) {
                    io::ErrorKind::PermissionDenied
                } else if err.is_instance_of::<PyIsADirectoryError>(py) {
                    io::ErrorKind::IsADirectory
                } else if err.is_instance_of::<PyOSError>(py) {
                    io::ErrorKind::Other
                } else {
                    let message = err.to_string();
                    keep(&self.raised, err);
                    return io::Error::other(message);
                };
                let message = err
                    .value(py)
                    .getattr("strerror")
                    .ok()
                    .and_then(|s| s.extract::<String>().ok())
                    .unwrap_or_else(|| err.to_string());
                io::Error::new(kind, message)
            })
        })
    }
}

impl FileLoader for PyLoader {
    fn read_file(&self, path: &str) -> io::Result<Vec<u8>> {
        self.read(self.files.as_ref(), path)
    }

    fn read_url(&self, url: &str) -> io::Result<Vec<u8>> {
        self.read(self.urls.as_ref(), url)
    }
}

// -- the parser -------------------------------------------------------------------

/// A configured parser. Use `lsdocs_rst.Parser` rather than this class.
#[pyclass(module = "lsdocs_rst._native", frozen)]
struct NativeParser {
    parser: Parser,
    raised: Raised,
}

#[pymethods]
impl NativeParser {
    /// `settings` is a dict of docutils setting names; `root` reads files
    /// below a directory, `files`/`urls` through callables.
    #[new]
    #[pyo3(signature = (*, source=None, transforms=false, keep_unknown_directives=false, settings=None, directives=None, roles=None, root=None, files=None, urls=None))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        source: Option<String>,
        transforms: bool,
        keep_unknown_directives: bool,
        settings: Option<&Bound<'_, PyAny>>,
        directives: Option<&Bound<'_, PyDict>>,
        roles: Option<&Bound<'_, PyDict>>,
        root: Option<std::path::PathBuf>,
        files: Option<Py<PyAny>>,
        urls: Option<Py<PyAny>>,
    ) -> PyResult<Self> {
        let raised: Raised = Arc::default();
        let mut parser = Parser::new()
            .transforms(transforms)
            .keep_unknown_directives(keep_unknown_directives);
        if let Some(source) = source {
            parser = parser.source(source);
        }
        if let Some(settings) = settings {
            let settings: Settings = depythonize(settings).map_err(value_error)?;
            parser = parser.settings(settings);
        }
        for (name, entry) in directives.into_iter().flat_map(|d| d.iter()) {
            let name: String = name.extract()?;
            let entry = entry.cast::<PyTuple>()?;
            let spec = directive_spec(entry.get_item(0)?.cast::<PyDict>()?)?;
            let run = entry.get_item(1)?.unbind();
            let raised = raised.clone();
            parser = parser.directive(&name, PyDirective { spec, run, raised });
        }
        for (name, run) in roles.into_iter().flat_map(|d| d.iter()) {
            let name: String = name.extract()?;
            let raised = raised.clone();
            parser = parser.role(
                &name,
                PyRole {
                    run: run.unbind(),
                    raised,
                },
            );
        }
        match (root, files.is_some() || urls.is_some()) {
            (Some(_), true) => {
                return Err(PyTypeError::new_err(
                    "give a root directory or loader functions, not both",
                ));
            }
            (Some(root), false) => {
                parser = parser.file_loader(lsdocs_rst::DirectoryLoader::new(root));
            }
            (None, true) => {
                let raised = raised.clone();
                parser = parser.file_loader(PyLoader {
                    files,
                    urls,
                    raised,
                });
            }
            (None, false) => {}
        }
        Ok(Self { parser, raised })
    }

    /// The settings in use, as a dict.
    fn settings<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        to_py(py, self.parser.current_settings())
    }

    /// Parses `text`: `(document, messages)` as dicts. A Python exception
    /// raised by a directive, role or loader is raised again here.
    fn parse<'py>(&self, py: Python<'py>, text: &str) -> PyResult<Bound<'py, PyTuple>> {
        let parser = self.parser.clone();
        let text = text.to_owned();
        let (document, messages) = py.detach(move || parser.parse_with_messages(&text));
        let first = self
            .raised
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
            .next();
        if let Some(err) = first {
            return Err(err);
        }
        PyTuple::new(py, [to_py(py, &document)?, to_py(py, &messages)?])
    }
}

// -- functions --------------------------------------------------------------------

/// The pseudo-XML of a tree given as a dict.
#[pyfunction]
fn pformat(tree: &Bound<'_, PyAny>) -> PyResult<String> {
    let node: Node = depythonize(tree).map_err(value_error)?;
    Ok(node.pformat())
}

/// The Docutils XML of an element given as a dict; `options` as
/// `lsdocs_rst::XmlOptions`.
#[pyfunction]
#[pyo3(signature = (tree, options=None))]
fn to_xml(tree: &Bound<'_, PyAny>, options: Option<&Bound<'_, PyAny>>) -> PyResult<String> {
    let element: Element = depythonize(tree).map_err(value_error)?;
    let options: XmlOptions = match options {
        Some(options) => depythonize(options).map_err(value_error)?,
        None => XmlOptions::default(),
    };
    Ok(element.to_xml(&options))
}

/// docutils' `make_id`.
#[pyfunction]
fn make_id(name: &str) -> String {
    lsdocs_rst::make_id(name)
}

/// docutils' defaults for the settings, as a dict.
#[pyfunction]
fn default_settings(py: Python<'_>) -> PyResult<Bound<'_, PyAny>> {
    to_py(py, &Settings::default())
}

#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<NativeParser>()?;
    m.add_class::<NativeContext>()?;
    m.add_function(wrap_pyfunction!(pformat, m)?)?;
    m.add_function(wrap_pyfunction!(to_xml, m)?)?;
    m.add_function(wrap_pyfunction!(make_id, m)?)?;
    m.add_function(wrap_pyfunction!(default_settings, m)?)?;
    Ok(())
}

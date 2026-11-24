//! Lua bindings for lsdocs-rst: the `lsdocs_rst` Lua module.
//!
//! ```lua
//! local rst = require("lsdocs_rst")
//! local doc, messages = rst.parse("Hello, *world*.\n", { transforms = true })
//! print(rst.pformat(doc))
//! ```
//!
//! Trees cross as plain Lua tables in the form of the `serde` feature of
//! `lsdocs-rst`: `{ kind, attributes, children, span }` for an element and
//! `{ text, span }` for text. A missing value (`span` of a node without a
//! source position, say) is `nil`, and arrays have no metatable. An
//! attribute value of Python's `None` (in `pending` details only) is
//! `lsdocs_rst.null`.
//!
//! Built with the `module` feature, this is a loadable module for an
//! external Lua 5.4 or LuaJIT. Without it (the default), the crate embeds
//! a vendored Lua 5.4 and [`open`] is what the tests call.

use lsdocs_rst::{Element, Node, Parser, Settings, XmlOptions};
use mlua::{
    Lua, LuaSerdeExt, MultiValue, Result, Table, UserData, UserDataMethods, Value,
    serde::SerializeOptions,
};

/// How values are turned into Lua: `nil` for `None` (a missing `span`, say),
/// and arrays without the array metatable, so trees are plain tables. An
/// attribute whose value is Python's `None` (only in `pending` details)
/// would vanish as `nil`, so it is `lsdocs_rst.null` (mlua's null
/// light userdata) instead.
fn options() -> SerializeOptions {
    SerializeOptions::new()
        .serialize_none_to_null(false)
        .serialize_unit_to_null(true)
        .set_array_metatable(false)
}

fn to_lua<T: serde::Serialize + ?Sized>(lua: &Lua, value: &T) -> Result<Value> {
    lua.to_value_with(value, options())
}

/// A tree (node, element, or list of them) from its table form.
///
/// An empty table is an empty array or an empty map; without the array
/// metatable Lua cannot say which, and the tree's untagged `Node` needs to
/// know. So the table goes through JSON first, where an empty `children`
/// or attribute value becomes an empty list (no attribute value is a map).
fn tree_from_lua<T: serde::de::DeserializeOwned>(lua: &Lua, value: Value) -> Result<T> {
    let mut json: serde_json::Value = lua.from_value(value)?;
    empty_lists(&mut json);
    serde_json::from_value(json).map_err(mlua::Error::external)
}

fn is_empty_object(value: &serde_json::Value) -> bool {
    value.as_object().is_some_and(serde_json::Map::is_empty)
}

fn empty_lists(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Array(items) => items.iter_mut().for_each(empty_lists),
        serde_json::Value::Object(node) => {
            if let Some(children) = node.get_mut("children") {
                if is_empty_object(children) {
                    *children = serde_json::Value::Array(Vec::new());
                }
                empty_lists(children);
            }
            if let Some(serde_json::Value::Object(attributes)) = node.get_mut("attributes") {
                for v in attributes.values_mut() {
                    if is_empty_object(v) {
                        *v = serde_json::Value::Array(Vec::new());
                    }
                }
            }
        }
        _ => {}
    }
}

/// A field of an options table, if the table and the field are given.
fn field<T: mlua::FromLua>(opts: Option<&Table>, name: &str) -> Result<Option<T>> {
    match opts {
        Some(opts) => opts.get(name),
        None => Ok(None),
    }
}

/// A parser from an options table: `source`, `transforms`,
/// `keep_unknown_directives`, `settings` (docutils setting names) and
/// `root` (a directory `include` and friends read below).
fn make_parser(lua: &Lua, opts: Option<Table>) -> Result<Parser> {
    let opts = opts.as_ref();
    let mut parser = Parser::new()
        .transforms(field(opts, "transforms")?.unwrap_or(false))
        .keep_unknown_directives(field(opts, "keep_unknown_directives")?.unwrap_or(false));
    if let Some(source) = field::<String>(opts, "source")? {
        parser = parser.source(source);
    }
    if let Some(settings) = field::<Value>(opts, "settings")? {
        let settings: Settings = lua.from_value(settings)?;
        parser = parser.settings(settings);
    }
    if let Some(root) = field::<String>(opts, "root")? {
        parser = parser.file_loader(lsdocs_rst::DirectoryLoader::new(root));
    }
    Ok(parser)
}

/// `document, messages` for `text`.
fn parse_with(lua: &Lua, parser: &Parser, text: &str) -> Result<MultiValue> {
    let (document, messages) = parser.parse_with_messages(text);
    Ok(MultiValue::from_vec(vec![
        to_lua(lua, &document)?,
        to_lua(lua, &messages)?,
    ]))
}

/// A configured parser, made by `lsdocs_rst.parser(options)`.
struct LuaParser(Parser);

impl UserData for LuaParser {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("parse", |lua, this, text: String| {
            parse_with(lua, &this.0, &text)
        });
        methods.add_method("settings", |lua, this, ()| {
            to_lua(lua, this.0.current_settings())
        });
    }
}

/// The module table.
///
/// # Errors
///
/// If Lua cannot allocate the table or its functions.
pub fn open(lua: &Lua) -> Result<Table> {
    let m = lua.create_table()?;
    m.set("version", env!("CARGO_PKG_VERSION"))?;
    m.set("null", lua.null())?;
    m.set(
        "parser",
        lua.create_function(|lua, opts: Option<Table>| Ok(LuaParser(make_parser(lua, opts)?)))?,
    )?;
    m.set(
        "parse",
        lua.create_function(|lua, (text, opts): (String, Option<Table>)| {
            parse_with(lua, &make_parser(lua, opts)?, &text)
        })?,
    )?;
    m.set(
        "pformat",
        lua.create_function(|lua, tree: Value| {
            let node: Node = tree_from_lua(lua, tree)?;
            Ok(node.pformat())
        })?,
    )?;
    m.set(
        "to_xml",
        lua.create_function(|lua, (tree, opts): (Value, Option<Value>)| {
            let element: Element = tree_from_lua(lua, tree)?;
            let opts: XmlOptions = match opts {
                Some(opts) => lua.from_value(opts)?,
                None => XmlOptions::default(),
            };
            Ok(element.to_xml(&opts))
        })?,
    )?;
    m.set(
        "make_id",
        lua.create_function(|_, name: String| Ok(lsdocs_rst::make_id(&name)))?,
    )?;
    m.set(
        "default_settings",
        lua.create_function(|lua, ()| to_lua(lua, &Settings::default()))?,
    )?;
    Ok(m)
}

/// The entry point an external interpreter calls (`luaopen_lsdocs_rst`).
#[cfg(feature = "module")]
#[mlua::lua_module(name = "lsdocs_rst")]
fn luaopen(lua: &Lua) -> Result<Table> {
    open(lua)
}

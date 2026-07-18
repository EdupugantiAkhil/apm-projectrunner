use appcui::prelude::*;
use serde_json::{Map, Value};

type SchemaValues = Map<String, Value>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SchemaFieldKind {
    Text,
    Integer,
    Number,
    Boolean,
    Enum(Vec<String>),
    Section,
    YamlFallback,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SchemaField {
    pub(crate) path: String,
    pub(crate) title: String,
    pub(crate) required: bool,
    pub(crate) kind: SchemaFieldKind,
}

pub(crate) fn classify_schema(schema: &Value) -> SchemaFieldKind {
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        let strings = values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if strings.len() == values.len() && !strings.is_empty() {
            return SchemaFieldKind::Enum(strings);
        }
        return SchemaFieldKind::YamlFallback;
    }
    match schema.get("type").and_then(Value::as_str) {
        Some("string") => SchemaFieldKind::Text,
        Some("integer") => SchemaFieldKind::Integer,
        Some("number") => SchemaFieldKind::Number,
        Some("boolean") => SchemaFieldKind::Boolean,
        Some("object")
            if schema
                .get("properties")
                .and_then(Value::as_object)
                .is_some() =>
        {
            SchemaFieldKind::Section
        }
        _ => SchemaFieldKind::YamlFallback,
    }
}

pub(crate) fn schema_fields(schema: &Value) -> Vec<SchemaField> {
    let mut fields = Vec::new();
    collect_fields(schema, schema, "", &mut fields);
    fields
}

fn collect_fields(root: &Value, schema: &Value, prefix: &str, fields: &mut Vec<SchemaField>) {
    let schema = resolve(root, schema).unwrap_or(schema);
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        fields.push(SchemaField {
            path: prefix.trim_matches('.').to_owned(),
            title: title(schema, prefix),
            required: false,
            kind: SchemaFieldKind::YamlFallback,
        });
        return;
    };
    let required = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|values| values.iter().filter_map(Value::as_str).collect::<Vec<_>>())
        .unwrap_or_default();
    for (name, child) in properties {
        let resolved = resolve(root, child).unwrap_or(child);
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}.{name}")
        };
        let kind = classify_schema(resolved);
        fields.push(SchemaField {
            path: path.clone(),
            title: resolved
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or(name)
                .to_owned(),
            required: required.contains(&name.as_str()),
            kind: kind.clone(),
        });
        if kind == SchemaFieldKind::Section {
            collect_fields(root, resolved, &path, fields);
        }
    }
}

fn resolve<'a>(root: &'a Value, schema: &'a Value) -> Option<&'a Value> {
    let reference = schema.get("$ref")?.as_str()?.strip_prefix("#/$defs/")?;
    root.get("$defs")?.get(reference)
}

fn title(schema: &Value, fallback: &str) -> String {
    schema
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or(if fallback.is_empty() {
            "Configuration"
        } else {
            fallback
        })
        .to_owned()
}

#[derive(Clone)]
struct EnumChoice(String);

impl DropDownListType for EnumChoice {
    fn name(&self) -> &str {
        &self.0
    }
}

enum FieldControl {
    Text(String, Handle<TextField>),
    Integer(String, Handle<NumericSelector<i64>>),
    Number(String, Handle<NumericSelector<f64>>),
    Boolean(String, Handle<CheckBox>),
    Enum(String, Handle<DropDownList<EnumChoice>>),
}

#[ModalWindow(events = ButtonEvents, response = SchemaValues)]
pub(crate) struct SchemaFormDialog {
    fields: Vec<FieldControl>,
    save: Handle<Button>,
    cancel: Handle<Button>,
}

impl SchemaFormDialog {
    pub(crate) fn new(title: &str, schema: &Value, initial: &Value) -> Self {
        let projected = schema_fields(schema);
        let has_fallback = projected
            .iter()
            .any(|field| field.kind == SchemaFieldKind::YamlFallback);
        let mut dialog = Self {
            base: ModalWindow::new(title, layout!("a:c,w:86,h:34"), window::Flags::None),
            fields: Vec::new(),
            save: Handle::None,
            cancel: Handle::None,
        };
        let mut y = 1;
        for field in projected {
            let value = value_at(initial, &field.path);
            let suffix = if field.required { " (required)" } else { "" };
            match field.kind {
                SchemaFieldKind::Section => {
                    dialog.add(Label::new(&field.title, Layout::absolute(2, y, 80, 1)));
                    y += 1;
                }
                SchemaFieldKind::Text => {
                    dialog.add(Label::new(
                        &format!("{}{suffix}", field.title),
                        Layout::absolute(2, y, 27, 1),
                    ));
                    let handle = dialog.add(TextField::new(
                        value.and_then(Value::as_str).unwrap_or(""),
                        Layout::absolute(30, y, 51, 1),
                        textfield::Flags::None,
                    ));
                    dialog.fields.push(FieldControl::Text(field.path, handle));
                    y += 2;
                }
                SchemaFieldKind::Integer => {
                    dialog.add(Label::new(
                        &format!("{}{suffix}", field.title),
                        Layout::absolute(2, y, 27, 1),
                    ));
                    let handle = dialog.add(NumericSelector::new(
                        value.and_then(Value::as_i64).unwrap_or_default(),
                        i64::MIN,
                        i64::MAX,
                        1,
                        Layout::absolute(30, y, 51, 1),
                        numericselector::Flags::None,
                    ));
                    dialog
                        .fields
                        .push(FieldControl::Integer(field.path, handle));
                    y += 2;
                }
                SchemaFieldKind::Number => {
                    dialog.add(Label::new(
                        &format!("{}{suffix}", field.title),
                        Layout::absolute(2, y, 27, 1),
                    ));
                    let handle = dialog.add(NumericSelector::new(
                        value.and_then(Value::as_f64).unwrap_or_default(),
                        -1.0e12,
                        1.0e12,
                        0.1,
                        Layout::absolute(30, y, 51, 1),
                        numericselector::Flags::None,
                    ));
                    dialog.fields.push(FieldControl::Number(field.path, handle));
                    y += 2;
                }
                SchemaFieldKind::Boolean => {
                    let mut control = CheckBox::new(
                        &format!("{}{suffix}", field.title),
                        Layout::absolute(2, y, 79, 1),
                        value.and_then(Value::as_bool).unwrap_or(false),
                    );
                    control.set_checked(value.and_then(Value::as_bool).unwrap_or(false));
                    let handle = dialog.add(control);
                    dialog
                        .fields
                        .push(FieldControl::Boolean(field.path, handle));
                    y += 2;
                }
                SchemaFieldKind::Enum(values) => {
                    dialog.add(Label::new(
                        &format!("{}{suffix}", field.title),
                        Layout::absolute(2, y, 27, 1),
                    ));
                    let mut control = DropDownList::new(
                        Layout::absolute(30, y, 51, 1),
                        dropdownlist::Flags::None,
                    );
                    let selected = value.and_then(Value::as_str);
                    for item in &values {
                        control.add(EnumChoice(item.clone()));
                    }
                    if let Some(index) = values
                        .iter()
                        .position(|item| Some(item.as_str()) == selected)
                    {
                        control.set_index(index as u32);
                    }
                    let handle = dialog.add(control);
                    dialog.fields.push(FieldControl::Enum(field.path, handle));
                    y += 2;
                }
                SchemaFieldKind::YamlFallback => {
                    dialog.add(Label::new(
                        &format!("{} — complex/unknown schema; read-only YAML", field.title),
                        Layout::absolute(2, y, 79, 1),
                    ));
                    y += 1;
                }
            }
        }
        if has_fallback {
            let yaml = serde_yaml::to_string(initial)
                .unwrap_or_else(|error| format!("# could not render YAML: {error}"));
            let top = y.min(24);
            dialog.add(Label::new(
                "Complex fields are read-only and shown as YAML:",
                Layout::absolute(2, top, 79, 1),
            ));
            dialog.add(TextArea::new(
                &yaml,
                Layout::absolute(2, top + 1, 79, 5),
                textarea::Flags::ReadOnly | textarea::Flags::ScrollBars,
            ));
        }
        dialog.save = dialog.add(Button::new(
            "&Preview",
            layout!("x:35%,y:100%,p:b,w:16,h:1"),
        ));
        dialog.cancel = dialog.add(Button::new("&Cancel", layout!("x:65%,y:100%,p:b,w:16,h:1")));
        dialog
    }

    fn values(&self) -> Map<String, Value> {
        let mut result = Map::new();
        for field in &self.fields {
            let (path, value) = match field {
                FieldControl::Text(path, h) => (
                    path,
                    self.control(*h).map(|c| Value::String(c.text().into())),
                ),
                FieldControl::Integer(path, h) => {
                    (path, self.control(*h).map(|c| Value::from(c.value())))
                }
                FieldControl::Number(path, h) => {
                    (path, self.control(*h).map(|c| Value::from(c.value())))
                }
                FieldControl::Boolean(path, h) => {
                    (path, self.control(*h).map(|c| Value::Bool(c.is_checked())))
                }
                FieldControl::Enum(path, h) => (
                    path,
                    self.control(*h)
                        .and_then(|c| c.selected_item())
                        .map(|c| Value::String(c.0.clone())),
                ),
            };
            if let Some(value) = value {
                result.insert(path.clone(), value);
            }
        }
        result
    }
}

impl ButtonEvents for SchemaFormDialog {
    fn on_pressed(&mut self, handle: Handle<Button>) -> EventProcessStatus {
        if handle == self.save {
            self.exit_with(self.values());
        } else if handle == self.cancel {
            self.exit();
        } else {
            return EventProcessStatus::Ignored;
        }
        EventProcessStatus::Processed
    }
}

fn value_at<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.')
        .try_fold(value, |current, segment| current.get(segment))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn schema_types_map_to_shared_controls() {
        assert_eq!(
            classify_schema(&json!({"type":"string"})),
            SchemaFieldKind::Text
        );
        assert_eq!(
            classify_schema(&json!({"type":"integer"})),
            SchemaFieldKind::Integer
        );
        assert_eq!(
            classify_schema(&json!({"type":"number"})),
            SchemaFieldKind::Number
        );
        assert_eq!(
            classify_schema(&json!({"type":"boolean"})),
            SchemaFieldKind::Boolean
        );
        assert_eq!(
            classify_schema(&json!({"enum":["a","b"]})),
            SchemaFieldKind::Enum(vec!["a".into(), "b".into()])
        );
        assert_eq!(
            classify_schema(&json!({"type":"array"})),
            SchemaFieldKind::YamlFallback
        );
    }

    #[test]
    fn nested_objects_become_sections_and_fields() {
        let fields = schema_fields(
            &json!({"type":"object","required":["name"],"properties":{"name":{"type":"string"},"build":{"type":"object","properties":{"port":{"type":"integer"}}}}}),
        );
        assert_eq!(
            fields
                .iter()
                .map(|f| (&*f.path, &f.kind))
                .collect::<Vec<_>>(),
            vec![
                ("build", &SchemaFieldKind::Section),
                ("build.port", &SchemaFieldKind::Integer),
                ("name", &SchemaFieldKind::Text)
            ]
        );
        assert!(
            fields
                .iter()
                .find(|field| field.path == "name")
                .unwrap()
                .required
        );
    }
}

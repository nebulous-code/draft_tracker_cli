//! The categories screen, which also manages types.
//!
//! Two panes side by side, because a type is a parent of a category and editing
//! one while unable to see the other makes the hierarchy hard to reason about.
//! Three types did not justify a screen of their own.

use super::entry::Mode;
use crate::db::repo::{CategoryRow, TypeRow};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Categories,
    Types,
}

impl Pane {
    pub fn other(self) -> Self {
        match self {
            Pane::Categories => Pane::Types,
            Pane::Types => Pane::Categories,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Pane::Categories => "Categories",
            Pane::Types => "Types",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CategoryField {
    Name,
    Type,
    Description,
}

impl CategoryField {
    pub const ORDER: [CategoryField; 3] = [
        CategoryField::Name,
        CategoryField::Type,
        CategoryField::Description,
    ];

    pub fn label(self) -> &'static str {
        match self {
            CategoryField::Name => "Name",
            CategoryField::Type => "Type",
            CategoryField::Description => "Description",
        }
    }

    fn index(self) -> usize {
        Self::ORDER
            .iter()
            .position(|f| *f == self)
            .expect("field is in ORDER")
    }

    pub fn next(self) -> Self {
        Self::ORDER[(self.index() + 1) % Self::ORDER.len()]
    }

    pub fn previous(self) -> Self {
        let i = self.index();
        Self::ORDER[(i + Self::ORDER.len() - 1) % Self::ORDER.len()]
    }

    pub fn is_choice(self) -> bool {
        matches!(self, CategoryField::Type)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeField {
    Name,
    Description,
}

impl TypeField {
    pub const ORDER: [TypeField; 2] = [TypeField::Name, TypeField::Description];

    pub fn label(self) -> &'static str {
        match self {
            TypeField::Name => "Name",
            TypeField::Description => "Description",
        }
    }

    pub fn next(self) -> Self {
        match self {
            TypeField::Name => TypeField::Description,
            TypeField::Description => TypeField::Name,
        }
    }

    pub fn previous(self) -> Self {
        self.next()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    EmptyName,
    NoTypes,
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Problem::EmptyName => write!(f, "a name is required"),
            Problem::NoTypes => {
                write!(
                    f,
                    "there are no types yet; create one in the types pane with o"
                )
            }
        }
    }
}

/// Whichever record is open for editing.
#[derive(Debug, Clone, PartialEq)]
pub enum Edit {
    Category(CategoryEdit),
    Type(TypeEdit),
}

impl Edit {
    pub fn mode(&self) -> Mode {
        match self {
            Edit::Category(e) => e.mode,
            Edit::Type(e) => e.mode,
        }
    }

    pub fn set_mode(&mut self, mode: Mode) {
        match self {
            Edit::Category(e) => e.mode = mode,
            Edit::Type(e) => e.mode = mode,
        }
    }

    pub fn is_changed(&self) -> bool {
        match self {
            Edit::Category(e) => e.is_changed(),
            Edit::Type(e) => e.is_changed(),
        }
    }

    pub fn is_creating(&self) -> bool {
        match self {
            Edit::Category(e) => e.id.is_none(),
            Edit::Type(e) => e.id.is_none(),
        }
    }

    pub fn field_label(&self) -> &'static str {
        match self {
            Edit::Category(e) => e.field.label(),
            Edit::Type(e) => e.field.label(),
        }
    }

    pub fn problems(&self) -> Vec<Problem> {
        match self {
            Edit::Category(e) => e.problems(),
            Edit::Type(e) => e.problems(),
        }
    }

    pub fn push_char(&mut self, c: char) {
        match self {
            Edit::Category(e) => e.push_char(c),
            Edit::Type(e) => e.push_char(c),
        }
    }

    pub fn backspace(&mut self) {
        match self {
            Edit::Category(e) => e.backspace(),
            Edit::Type(e) => e.backspace(),
        }
    }

    pub fn next_field(&mut self) {
        match self {
            Edit::Category(e) => e.field = e.field.next(),
            Edit::Type(e) => e.field = e.field.next(),
        }
    }

    pub fn previous_field(&mut self) {
        match self {
            Edit::Category(e) => e.field = e.field.previous(),
            Edit::Type(e) => e.field = e.field.previous(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CategoryEdit {
    pub id: Option<i64>,
    pub name: String,
    pub type_id: Option<i64>,
    pub description: String,
    pub field: CategoryField,
    pub mode: Mode,
    original: (String, Option<i64>, String),
}

impl CategoryEdit {
    pub fn existing(row: &CategoryRow) -> Self {
        let name = row.category.name.clone();
        let description = row.category.description.clone().unwrap_or_default();
        Self {
            id: Some(row.category.id),
            name: name.clone(),
            type_id: Some(row.category.type_id),
            description: description.clone(),
            field: CategoryField::Name,
            mode: Mode::Normal,
            original: (name, Some(row.category.type_id), description),
        }
    }

    pub fn creating(type_id: Option<i64>) -> Self {
        Self {
            id: None,
            name: String::new(),
            type_id,
            description: String::new(),
            field: CategoryField::Name,
            mode: Mode::Insert,
            original: (String::new(), type_id, String::new()),
        }
    }

    pub fn is_changed(&self) -> bool {
        self.original != (self.name.clone(), self.type_id, self.description.clone())
    }

    pub fn text_of(&self, field: CategoryField) -> &str {
        match field {
            CategoryField::Name => &self.name,
            CategoryField::Description => &self.description,
            CategoryField::Type => "",
        }
    }

    fn text_mut(&mut self) -> Option<&mut String> {
        match self.field {
            CategoryField::Name => Some(&mut self.name),
            CategoryField::Description => Some(&mut self.description),
            CategoryField::Type => None,
        }
    }

    pub fn push_char(&mut self, c: char) {
        if self.mode == Mode::Insert
            && let Some(text) = self.text_mut()
        {
            text.push(c);
        }
    }

    pub fn backspace(&mut self) {
        if self.mode == Mode::Insert
            && let Some(text) = self.text_mut()
        {
            text.pop();
        }
    }

    pub fn problems(&self) -> Vec<Problem> {
        let mut problems = Vec::new();
        if self.name.trim().is_empty() {
            problems.push(Problem::EmptyName);
        }
        // A category without a type would leave the hierarchy incomplete, and
        // the schema requires one.
        if self.type_id.is_none() {
            problems.push(Problem::NoTypes);
        }
        problems
    }

    pub fn description_option(&self) -> Option<String> {
        let trimmed = self.description.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypeEdit {
    pub id: Option<i64>,
    pub name: String,
    pub description: String,
    pub field: TypeField,
    pub mode: Mode,
    original: (String, String),
}

impl TypeEdit {
    pub fn existing(row: &TypeRow) -> Self {
        let name = row.type_.name.clone();
        let description = row.type_.description.clone().unwrap_or_default();
        Self {
            id: Some(row.type_.id),
            name: name.clone(),
            description: description.clone(),
            field: TypeField::Name,
            mode: Mode::Normal,
            original: (name, description),
        }
    }

    pub fn creating() -> Self {
        Self {
            id: None,
            name: String::new(),
            description: String::new(),
            field: TypeField::Name,
            mode: Mode::Insert,
            original: (String::new(), String::new()),
        }
    }

    pub fn is_changed(&self) -> bool {
        self.original != (self.name.clone(), self.description.clone())
    }

    pub fn text_of(&self, field: TypeField) -> &str {
        match field {
            TypeField::Name => &self.name,
            TypeField::Description => &self.description,
        }
    }

    fn text_mut(&mut self) -> &mut String {
        match self.field {
            TypeField::Name => &mut self.name,
            TypeField::Description => &mut self.description,
        }
    }

    pub fn push_char(&mut self, c: char) {
        if self.mode == Mode::Insert {
            self.text_mut().push(c);
        }
    }

    pub fn backspace(&mut self) {
        if self.mode == Mode::Insert {
            self.text_mut().pop();
        }
    }

    pub fn problems(&self) -> Vec<Problem> {
        if self.name.trim().is_empty() {
            vec![Problem::EmptyName]
        } else {
            Vec::new()
        }
    }

    pub fn description_option(&self) -> Option<String> {
        let trimmed = self.description.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    }
}

pub struct Categories {
    categories: Vec<CategoryRow>,
    types: Vec<TypeRow>,
    pub pane: Pane,
    pub mode: Mode,
    pub edit: Option<Edit>,
    selected_category: usize,
    selected_type: usize,
    pub confirming_delete: bool,
    pending_d: bool,
}

impl Categories {
    pub fn new(categories: Vec<CategoryRow>, types: Vec<TypeRow>) -> Self {
        Self {
            categories,
            types,
            pane: Pane::Categories,
            mode: Mode::Normal,
            edit: None,
            selected_category: 0,
            selected_type: 0,
            confirming_delete: false,
            pending_d: false,
        }
    }

    pub fn refresh(&mut self, categories: Vec<CategoryRow>, types: Vec<TypeRow>) {
        let category = self.selected_category().map(|r| r.category.id);
        let type_ = self.selected_type().map(|r| r.type_.id);
        self.categories = categories;
        self.types = types;
        if let Some(id) = category
            && let Some(i) = self.categories.iter().position(|r| r.category.id == id)
        {
            self.selected_category = i;
        }
        if let Some(id) = type_
            && let Some(i) = self.types.iter().position(|r| r.type_.id == id)
        {
            self.selected_type = i;
        }
        self.selected_category = self
            .selected_category
            .min(self.categories.len().saturating_sub(1));
        self.selected_type = self.selected_type.min(self.types.len().saturating_sub(1));
    }

    pub fn category_rows(&self) -> &[CategoryRow] {
        &self.categories
    }

    pub fn type_rows(&self) -> &[TypeRow] {
        &self.types
    }

    pub fn selected_category_index(&self) -> usize {
        self.selected_category
    }

    pub fn selected_type_index(&self) -> usize {
        self.selected_type
    }

    pub fn selected_category(&self) -> Option<&CategoryRow> {
        self.categories.get(self.selected_category)
    }

    pub fn selected_type(&self) -> Option<&TypeRow> {
        self.types.get(self.selected_type)
    }

    pub fn type_name(&self, id: Option<i64>) -> String {
        match id {
            None => "—".to_string(),
            Some(id) => self
                .types
                .iter()
                .find(|t| t.type_.id == id)
                .map(|t| t.type_.name.clone())
                .unwrap_or_else(|| "—".to_string()),
        }
    }

    fn len(&self) -> usize {
        match self.pane {
            Pane::Categories => self.categories.len(),
            Pane::Types => self.types.len(),
        }
    }

    fn cursor_mut(&mut self) -> &mut usize {
        match self.pane {
            Pane::Categories => &mut self.selected_category,
            Pane::Types => &mut self.selected_type,
        }
    }

    pub fn select_next(&mut self) {
        let len = self.len();
        if len > 0 {
            let cursor = self.cursor_mut();
            *cursor = (*cursor + 1) % len;
        }
    }

    pub fn select_previous(&mut self) {
        let len = self.len();
        if len > 0 {
            let cursor = self.cursor_mut();
            *cursor = (*cursor + len - 1) % len;
        }
    }

    pub fn select_first(&mut self) {
        *self.cursor_mut() = 0;
    }

    pub fn select_last(&mut self) {
        let last = self.len().saturating_sub(1);
        *self.cursor_mut() = last;
    }

    pub fn switch_pane(&mut self) {
        self.pane = self.pane.other();
    }

    pub fn begin_edit(&mut self) -> bool {
        match self.pane {
            Pane::Categories => match self.selected_category() {
                Some(row) => {
                    self.edit = Some(Edit::Category(CategoryEdit::existing(row)));
                    true
                }
                None => false,
            },
            Pane::Types => match self.selected_type() {
                Some(row) => {
                    self.edit = Some(Edit::Type(TypeEdit::existing(row)));
                    true
                }
                None => false,
            },
        }
    }

    pub fn begin_create(&mut self) {
        self.edit = Some(match self.pane {
            // A new category starts on whichever type is in view, which is
            // usually the one it belongs under.
            Pane::Categories => Edit::Category(CategoryEdit::creating(
                self.selected_type()
                    .map(|t| t.type_.id)
                    .or(self.types.first().map(|t| t.type_.id)),
            )),
            Pane::Types => Edit::Type(TypeEdit::creating()),
        });
    }

    pub fn cancel_edit(&mut self) -> bool {
        match self.edit.take() {
            Some(edit) => edit.is_changed(),
            None => false,
        }
    }

    pub fn has_unsaved_input(&self) -> bool {
        self.edit.as_ref().is_some_and(|e| e.is_changed())
    }

    /// Step a category's type. Only the type field is a choice.
    pub fn cycle_choice(&mut self, step: i64) {
        let ids: Vec<i64> = self.types.iter().map(|t| t.type_.id).collect();
        if ids.is_empty() {
            return;
        }
        let Some(Edit::Category(edit)) = self.edit.as_mut() else {
            return;
        };
        if edit.field != CategoryField::Type {
            return;
        }
        let current = edit
            .type_id
            .and_then(|id| ids.iter().position(|i| *i == id))
            .unwrap_or(0) as i64;
        let len = ids.len() as i64;
        let next = ((current + step) % len + len) % len;
        edit.type_id = Some(ids[next as usize]);
    }

    pub fn press_d(&mut self) -> bool {
        if self.pending_d {
            self.pending_d = false;
            true
        } else {
            self.pending_d = true;
            false
        }
    }

    pub fn clear_pending_d(&mut self) {
        self.pending_d = false;
    }

    pub fn pending_d(&self) -> bool {
        self.pending_d
    }
}

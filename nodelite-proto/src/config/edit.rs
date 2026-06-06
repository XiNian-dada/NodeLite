//! TOML edit helpers used when persisting validated config changes.
//!
//! Settings writes should preserve user-authored comments and section layout where possible,
//! while still replacing stale values that no longer exist in the validated config view.

use toml_edit::{ArrayOfTables, Item, Table, Value};

/// Insert or merge a top-level TOML item while preserving existing value decor.
pub fn upsert_toml_item_preserving_decor(root: &mut Table, key: &str, replacement: Item) {
    if let Some(existing) = root.get_mut(key) {
        merge_item(existing, replacement);
    } else {
        root.insert(key, replacement);
    }
}

fn merge_item(existing: &mut Item, replacement: Item) {
    match replacement {
        Item::None => *existing = Item::None,
        Item::Value(value) => merge_value(existing, value),
        Item::Table(table) => merge_table_item(existing, table),
        Item::ArrayOfTables(tables) => merge_array_of_tables_item(existing, tables),
    }
}

fn merge_value(existing: &mut Item, replacement: Value) {
    let Some(existing_value) = existing.as_value_mut() else {
        *existing = Item::Value(replacement);
        return;
    };
    let decor = existing_value.decor().clone();
    *existing_value = replacement;
    *existing_value.decor_mut() = decor;
}

fn merge_table_item(existing: &mut Item, replacement: Table) {
    let Some(existing_table) = existing.as_table_mut() else {
        *existing = Item::Table(replacement);
        return;
    };
    merge_table(existing_table, replacement);
}

fn merge_table(existing: &mut Table, replacement: Table) {
    let mut stale_keys = existing
        .iter()
        .map(|(key, _)| key.to_string())
        .collect::<Vec<_>>();

    for (key, replacement_item) in replacement {
        let key = key.to_string();
        if let Some(existing_item) = existing.get_mut(&key) {
            merge_item(existing_item, replacement_item);
        } else {
            existing.insert(&key, replacement_item);
        }
        stale_keys.retain(|stale_key| stale_key != &key);
    }

    for key in stale_keys {
        existing.remove(&key);
    }
}

fn merge_array_of_tables_item(existing: &mut Item, replacement: ArrayOfTables) {
    let Some(existing_tables) = existing.as_array_of_tables_mut() else {
        *existing = Item::ArrayOfTables(replacement);
        return;
    };
    merge_array_of_tables(existing_tables, replacement);
}

fn merge_array_of_tables(existing: &mut ArrayOfTables, replacement: ArrayOfTables) {
    if all_tables_have_id(existing) && all_tables_have_id(&replacement) {
        merge_array_of_tables_by_id(existing, replacement);
        return;
    }

    let replacement_len = replacement.len();
    for (index, replacement_table) in replacement.into_iter().enumerate() {
        if let Some(existing_table) = existing.get_mut(index) {
            merge_table(existing_table, replacement_table);
        } else {
            existing.push(replacement_table);
        }
    }
    while existing.len() > replacement_len {
        existing.remove(existing.len() - 1);
    }
}

fn merge_array_of_tables_by_id(existing: &mut ArrayOfTables, replacement: ArrayOfTables) {
    let mut merged = ArrayOfTables::new();
    for replacement_table in replacement {
        let id = table_id(&replacement_table).map(ToOwned::to_owned);
        if let Some(index) = id
            .as_deref()
            .and_then(|id| find_table_index_by_id(existing, id))
        {
            if let Some(mut existing_table) = existing.get(index).cloned() {
                existing.remove(index);
                merge_table(&mut existing_table, replacement_table);
                merged.push(existing_table);
            } else {
                merged.push(replacement_table);
            }
        } else {
            merged.push(replacement_table);
        }
    }
    while !existing.is_empty() {
        existing.remove(existing.len() - 1);
    }
    for table in merged {
        existing.push(table);
    }
}

fn all_tables_have_id(tables: &ArrayOfTables) -> bool {
    tables.iter().all(|table| table_id(table).is_some())
}

fn find_table_index_by_id(tables: &ArrayOfTables, id: &str) -> Option<usize> {
    tables
        .iter()
        .enumerate()
        .find_map(|(index, table)| (table_id(table) == Some(id)).then_some(index))
}

fn table_id(table: &Table) -> Option<&str> {
    table.get("id")?.as_value()?.as_str()
}

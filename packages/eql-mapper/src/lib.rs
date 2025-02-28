//! `eql-mapper` transforms SQL to SQL+EQL using a known database schema as a reference.

mod eql_mapper;
mod importer;
mod inference;
mod iterator_ext;
mod model;
mod scope;

pub use eql_mapper::*;
pub use importer::*;
pub use inference::*;
pub use model::*;
pub use scope::*;

// pub mod provenance_tracker;
// pub mod resolver;

// #[cfg(test)]
// mod tests {
//     use std::{cell::RefCell, rc::Rc, sync::Arc};

//     use bigdecimal::BigDecimal;
//     use pretty_assertions::assert_eq;
//     use sqlparser::{
//         ast::{Expr, Ident, Statement, Value},
//         dialect::GenericDialect,
//         parser::Parser,
//     };

//     use sqltk::prelude::*;

//     use crate::{
//         make_schema,
//         model::{
//             DeleteProvenance, InsertProvenance, ProjectionColumn, Provenance, Schema, SchemaError,
//             ScopeError, ScopeTracker, SelectProvenance, TableColumn,
//         },
//         rc::ViralRc,
//         visitors::{
//             importer::{ImportError, Importer},
//             provenance_tracker::ProvenanceError,
//             resolver::ResolverError,
//         },
//     };

//     use super::{provenance_tracker::ProvenanceTracker, resolver::Resolver};

//     /// Creates an unquoted identifier.
//     fn id(name: &str) -> Ident {
//         let mut ident = Ident::new(name);
//         ident.quote_style = None;
//         ident
//     }

//     /// Creates an quoted identifier.
//     fn idq(name: &str) -> Ident {
//         let mut ident = Ident::new(name);
//         ident.quote_style = Some('"');
//         ident
//     }

//     fn parse_sql(sql: &str) -> Vec<Statement> {
//         let dialect = GenericDialect {};
//         Parser::parse_sql(&dialect, sql).unwrap()
//     }

//     struct TestVisitor<'a> {
//         scope: Rc<RefCell<ScopeTracker<'a>>>,
//         importer: Rc<RefCell<Importer<'a>>>,
//         resolver: Rc<RefCell<Resolver<'a>>>,
//         provenance_tracker: Rc<RefCell<ProvenanceTracker<'a>>>,
//     }

//     impl<'a> TestVisitor<'a> {
//         fn new(schema: Arc<Schema>, ast_root: ViralRc<'a, Statement>) -> Self {
//             let scope = ScopeTracker::new(ast_root.clone());
//             let resolver = Resolver::new(schema.clone(), scope.clone(), ast_root.clone());
//             let importer = Importer::new(
//                 schema.clone(),
//                 ast_root.clone(),
//                 scope.clone(),
//                 resolver.clone(),
//             );
//             let provenance_tracker =
//                 ProvenanceTracker::new(schema.clone(), resolver.clone(), importer.clone());

//             Self {
//                 scope,
//                 importer,
//                 resolver,
//                 provenance_tracker,
//             }
//         }
//     }

//     #[derive(Debug, thiserror::Error)]
//     enum TestVisitorError {
//         #[error(transparent)]
//         Scope(#[from] ScopeError),

//         #[error(transparent)]
//         Import(#[from] ImportError),

//         #[error(transparent)]
//         Resolver(#[from] ResolverError),

//         #[error(transparent)]
//         Provenance(#[from] ProvenanceError),
//     }

//     impl<'ast> Visitor<'ast> for TestVisitor<'ast> {
//         type Error = TestVisitorError;

//         fn enter<N: Visitable>(&mut self, node: &'ast N) -> ControlFlow<Break<Self::Error>> {
//             convert_control_flow(self.provenance_tracker.borrow_mut().enter(node))?;
//             convert_control_flow(self.resolver.borrow_mut().enter(node))?;
//             convert_control_flow(self.importer.borrow_mut().enter(node))?;
//             convert_control_flow(self.scope.borrow_mut().enter(node))
//         }

//         fn exit<N: Visitable>(&mut self, node: &'ast N) -> ControlFlow<Break<Self::Error>> {
//             convert_control_flow(self.scope.borrow_mut().exit(node))?;
//             convert_control_flow(self.importer.borrow_mut().exit(node))?;
//             convert_control_flow(self.resolver.borrow_mut().exit(node))?;
//             convert_control_flow(self.provenance_tracker.borrow_mut().exit(node))
//         }
//     }

//     #[test]
//     fn select_one_column_from_one_table() {
//         let schema = make_schema! {
//             tables: {
//                 users: {
//                     id (PK),
//                     email,
//                     first_name,
//                 }
//             }
//         };

//         let user_id_column = schema
//             .resolve_table_column(&id("users"), &id("id"))
//             .unwrap();

//         let statements = parse_sql("select id from users;");
//         let test_visitor = TestVisitor::new(&schema);

//         match statements.accept(&test_visitor) {
//             ControlFlow::Continue(()) => match test_visitor
//                 .provenance_tracker
//                 .statement_provenance
//                 .get_tag(&statements[0])
//                 .unwrap()
//             {
//                 Provenance::Select(SelectProvenance {
//                     projection_table_columns,
//                     ..
//                 }) => {
//                     assert_eq!(projection_table_columns, &vec![user_id_column]);
//                 }
//                 other => panic!("Unexpected provenance: {:#?}", other),
//             },
//             ControlFlow::Break(Break::Err(err)) => {
//                 panic!("Error during AST evaluation: {:#?}", err);
//             }
//             other => panic!("Unexpected control flow: {:#?}", other),
//         }
//     }

//     #[test]
//     fn select_columns_from_multiple_tables() {
//         let schema = make_schema! {
//             tables: {
//                 users: {
//                     id (PK),
//                     email,
//                     first_name,
//                 }
//                 todo_lists: {
//                     id (PK),
//                     name,
//                     owner_id,
//                     created_at,
//                     updated_at,
//                 }
//             }
//         };

//         let user_id_column = schema
//             .resolve_table_column(&id("users"), &id("id"))
//             .unwrap();

//         let statements = parse_sql(
//             "select u.id from users as u inner join todo_lists as tl on tl.owner_id = u.id;",
//         );

//         let test_visitor = TestVisitor::new(&schema);

//         match statements.accept(&test_visitor) {
//             ControlFlow::Continue(()) => match test_visitor
//                 .provenance_tracker
//                 .statement_provenance
//                 .get_tag(&statements[0])
//                 .unwrap()
//             {
//                 Provenance::Select(SelectProvenance {
//                     projection_table_columns,
//                     ..
//                 }) => {
//                     assert_eq!(projection_table_columns, &vec![user_id_column]);
//                 }
//                 other => panic!("Unexpected provenance: {:#?}", other),
//             },
//             ControlFlow::Break(Break::Err(err)) => {
//                 panic!("Error during AST evaluation: {:#?}", err);
//             }
//             other => panic!("Unexpected control flow: {:#?}", other),
//         }
//     }

//     #[test]
//     fn select_columns_from_subquery() -> Result<(), SchemaError> {
//         let schema = make_schema! {
//             tables: {
//                 users: {
//                     id,
//                     email,
//                     first_name,
//                 }
//                 todo_lists: {
//                     id,
//                     name,
//                     owner_id,
//                     created_at,
//                     updated_at,
//                 }
//                 todo_list_items: {
//                     id,
//                     description,
//                     owner_id,
//                     created_at,
//                     updated_at,
//                 }
//             }
//         };

//         let user_id_column = schema.resolve_table_column(&id("users"), &id("id"))?;

//         let todo_list_items_id_column =
//             schema.resolve_table_column(&id("todo_list_items"), &id("id"))?;

//         let todo_list_items_description_column =
//             schema.resolve_table_column(&id("todo_list_items"), &id("description"))?;

//         let statements = parse_sql(
//             r#"
//                 select
//                     u.id as user_id,
//                     tli.id as todo_list_item_id,
//                     tli.description as todo_list_item_description
//                 from
//                     users as u
//                 inner join (
//                     select
//                         id,
//                         owner_id,
//                         description
//                     from
//                         todo_list_items
//                 ) as tli on tli.owner_id = u.id;
//             "#,
//         );

//         let test_visitor = TestVisitor::new(&schema);

//         match statements.accept(&test_visitor) {
//             ControlFlow::Continue(()) => match test_visitor
//                 .provenance_tracker
//                 .statement_provenance
//                 .get_tag(&statements[0])
//                 .unwrap()
//             {
//                 Provenance::Select(SelectProvenance { projection, .. }) => {
//                     let columns = projection.columns_iter().collect::<Vec<_>>();
//                     assert_eq!(
//                         columns[0],
//                         &ProjectionColumn::TableColumn(user_id_column, Some(id("user_id"))),
//                     );
//                     assert_eq!(
//                         columns[1],
//                         &ProjectionColumn::TableColumn(
//                             todo_list_items_id_column,
//                             Some(id("todo_list_item_id"))
//                         ),
//                     );
//                     assert_eq!(
//                         columns[2],
//                         &ProjectionColumn::TableColumn(
//                             todo_list_items_description_column,
//                             Some(id("todo_list_item_description"))
//                         ),
//                     );

//                     Ok(())
//                 }
//                 other => panic!("Unexpected provenance: {:#?}", other),
//             },
//             ControlFlow::Break(Break::Err(err)) => {
//                 panic!("Error during AST evaluation: {:#?}", err);
//             }
//             other => panic!("Unexpected control flow: {:#?}", other),
//         }
//     }

//     #[test]
//     fn select_columns_from_correlated_subquery() -> Result<(), SchemaError> {
//         let schema = make_schema! {
//             tables: {
//                 films: {
//                     id,
//                     title,
//                     length,
//                     rating,
//                 }
//             }
//         };

//         let films_id_column = schema
//             .resolve_table_column(&id("films"), &id("id"))
//             .unwrap();
//         let films_title_column = schema
//             .resolve_table_column(&id("films"), &id("title"))
//             .unwrap();
//         let films_length_column = schema
//             .resolve_table_column(&id("films"), &id("length"))
//             .unwrap();
//         let films_rating_column = schema
//             .resolve_table_column(&id("films"), &id("rating"))
//             .unwrap();

//         let statements = parse_sql(
//             r#"
//             select f.id, f.title, f.length, f.rating
//             from films f
//             where length > (
//                 select avg(length)
//                 from films
//                 where rating = f.rating
//             );
//         "#,
//         );

//         let test_visitor = TestVisitor::new(&schema);

//         match statements.accept(&test_visitor) {
//             ControlFlow::Continue(()) => match test_visitor
//                 .provenance_tracker
//                 .statement_provenance
//                 .get_tag(&statements[0])
//                 .unwrap()
//             {
//                 Provenance::Select(SelectProvenance { projection, .. }) => {
//                     let columns = projection.columns_iter().collect::<Vec<_>>();
//                     assert_eq!(
//                         columns[0],
//                         &ProjectionColumn::TableColumn(films_id_column, Some(idq("id"))),
//                     );
//                     assert_eq!(
//                         columns[1],
//                         &ProjectionColumn::TableColumn(films_title_column, Some(idq("title"))),
//                     );
//                     assert_eq!(
//                         columns[2],
//                         &ProjectionColumn::TableColumn(films_length_column, Some(idq("length"))),
//                     );
//                     assert_eq!(
//                         columns[3],
//                         &ProjectionColumn::TableColumn(films_rating_column, Some(idq("rating"))),
//                     );

//                     Ok(())
//                 }
//                 other => panic!("Unexpected provenance: {:#?}", other),
//             },
//             ControlFlow::Break(Break::Err(err)) => {
//                 panic!("Error during AST evaluation: {:#?}", err);
//             }
//             other => panic!("Unexpected control flow: {:#?}", other),
//         }
//     }

//     #[test]
//     fn select_columns_from_cte() -> Result<(), SchemaError> {
//         let schema = make_schema! { name: "public" };

//         let statements = parse_sql(
//             r#"
//                 with some_cte as (
//                     select 123 as id
//                 )
//                 select id from some_cte;
//             "#,
//         );

//         let test_visitor = TestVisitor::new(&schema);

//         match statements.accept(&test_visitor) {
//             ControlFlow::Continue(()) => match test_visitor
//                 .provenance_tracker
//                 .statement_provenance
//                 .get_tag(&statements[0])
//                 .unwrap()
//             {
//                 Provenance::Select(SelectProvenance { projection, .. }) => {
//                     let columns = projection.columns_iter().collect::<Vec<_>>();
//                     assert_eq!(
//                         columns[0],
//                         &ProjectionColumn::Expr(
//                             &Expr::Value(Value::Number(BigDecimal::from(123), false)),
//                             Some(id("id"))
//                         ),
//                     );

//                     Ok(())
//                 }
//                 other => panic!("Unexpected provenance: {:#?}", other),
//             },
//             ControlFlow::Break(Break::Err(err)) => {
//                 panic!("Error during AST evaluation: {:#?}", err);
//             }
//             other => panic!("Unexpected control flow: {:#?}", other),
//         }
//     }

//     #[test]
//     fn basic_insert() -> Result<(), SchemaError> {
//         let schema = make_schema! {
//             tables: {
//                 films: {
//                     id,
//                     title,
//                     length,
//                     rating,
//                 }
//             }
//         };

//         let statements = parse_sql(
//             r#"
//             insert into films (title, length, rating)
//                 values ('Star Wars', '2 hours', '10/10')
//                 returning id;
//         "#,
//         );

//         let films_id_column = schema
//             .resolve_table_column(&id("films"), &id("id"))
//             .unwrap();

//         let test_visitor = TestVisitor::new(&schema);

//         match statements.accept(&test_visitor) {
//             ControlFlow::Continue(()) => match test_visitor
//                 .provenance_tracker
//                 .statement_provenance
//                 .get_tag(&statements[0])
//                 .unwrap()
//             {
//                 Provenance::Insert(InsertProvenance {
//                     into_table,
//                     returning: Some(returning),
//                     returning_table_columns: Some(returning_table_columns),
//                     columns_written,
//                     source_projection: Some(_),
//                     source_table_columns: Some(_),
//                 }) => {
//                     assert_eq!(*into_table, schema.resolve_table(&id("films"))?);
//                     let columns = returning.columns_iter().collect::<Vec<_>>();
//                     assert_eq!(
//                         columns[0],
//                         &ProjectionColumn::TableColumn(films_id_column.clone(), Some(idq("id")))
//                     );
//                     assert_eq!(returning_table_columns, &vec![films_id_column],);
//                     assert_eq!(
//                         columns_written,
//                         &vec![
//                             TableColumn {
//                                 table: into_table.clone(),
//                                 column: into_table.get_column(&id("title"))?,
//                             },
//                             TableColumn {
//                                 table: into_table.clone(),
//                                 column: into_table.get_column(&id("length"))?,
//                             },
//                             TableColumn {
//                                 table: into_table.clone(),
//                                 column: into_table.get_column(&id("rating"))?
//                             }
//                         ]
//                     );

//                     Ok(())
//                 }
//                 other => panic!("Unexpected provenance: {:#?}", other),
//             },
//             ControlFlow::Break(Break::Err(err)) => {
//                 panic!("Error during AST evaluation: {:#?}", err);
//             }
//             other => panic!("Unexpected control flow: {:#?}", other),
//         }
//     }

//     #[test]
//     fn basic_delete() -> Result<(), SchemaError> {
//         let schema = make_schema! {
//             tables: {
//                 films: {
//                     id,
//                     title,
//                     length,
//                     rating,
//                 }
//             }
//         };

//         let statements = parse_sql("delete from films where id = 123 returning id;");

//         let films_id_column = schema
//             .resolve_table_column(&id("films"), &id("id"))
//             .unwrap();

//         let test_visitor = TestVisitor::new(&schema);

//         match statements.accept(&test_visitor) {
//             ControlFlow::Continue(()) => match test_visitor
//                 .provenance_tracker
//                 .statement_provenance
//                 .get_tag(&statements[0])
//                 .unwrap()
//             {
//                 Provenance::Delete(DeleteProvenance {
//                     from_table,
//                     returning: Some(returning),
//                     returning_table_columns: Some(returning_table_columns),
//                 }) => {
//                     assert_eq!(*from_table, schema.resolve_table(&id("films"))?);
//                     let columns = returning.columns_iter().collect::<Vec<_>>();
//                     assert_eq!(
//                         columns[0],
//                         &ProjectionColumn::TableColumn(films_id_column.clone(), Some(idq("id")))
//                     );
//                     assert_eq!(
//                         returning_table_columns,
//                         &vec![TableColumn {
//                             table: from_table.clone(),
//                             column: from_table.get_column(&id("id"))?
//                         }]
//                     );

//                     Ok(())
//                 }
//                 other => panic!("Unexpected provenance: {:#?}", other),
//             },
//             ControlFlow::Break(Break::Err(err)) => {
//                 panic!("Error during AST evaluation: {:#?}", err);
//             }
//             other => panic!("Unexpected control flow: {:#?}", other),
//         }
//     }
// }

use diesel::prelude::*;

#[derive(Queryable)]
#[diesel(table_name = sessions)]
struct Session {
    id: i64,
}

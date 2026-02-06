use super::Message;
use crate::utils::AppError;
use plast_mem_db_schema::message_queue;
use sea_orm::{
  ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, ExprTrait, QueryFilter, Set,
  TransactionTrait,
  prelude::Expr,
  sea_query::{BinOper, extension::postgres::PgBinOper},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MessageQueue {
  pub id: Uuid,
  pub messages: Vec<Message>,
}

impl MessageQueue {
  pub async fn get(id: Uuid, db: &DatabaseConnection) -> Result<Self, AppError> {
    let model = Self::get_or_create_model(id, db).await?;

    Ok(Self::from_model(model)?)
  }

  pub async fn get_or_create_model(
    id: Uuid,
    db: &DatabaseConnection,
  ) -> Result<message_queue::Model, AppError> {
    let txn = db.begin().await?;

    let queue = match message_queue::Entity::find_by_id(id).one(&txn).await? {
      Some(model) => model,
      None => {
        let active_model = message_queue::ActiveModel {
          id: Set(id),
          messages: Set(serde_json::to_value(Vec::<Message>::new())?),
        };

        active_model.insert(&txn).await?
      }
    };

    txn.commit().await?;
    Ok(queue)
  }

  pub fn from_model(model: message_queue::Model) -> Result<Self, AppError> {
    Ok(Self {
      id: model.id,
      messages: serde_json::from_value(model.messages)?,
    })
  }

  // pub fn to_model(&self) -> Result<message_queue::Model, AppError> {
  //   Ok(message_queue::Model {
  //     id: self.id,
  //     messages: serde_json::to_value(&self.messages)?,
  //   })
  // }

  pub async fn push(id: Uuid, message: Message, db: &DatabaseConnection) -> Result<(), AppError> {
    let message_value = serde_json::to_value(message)?;

    message_queue::Entity::update_many()
      .col_expr(
        message_queue::Column::Messages,
        Expr::col(message_queue::Column::Messages).binary(
          BinOper::PgOperator(PgBinOper::Concatenate),
          Expr::val(message_value),
        ),
      )
      .filter(message_queue::Column::Id.eq(id))
      .exec(db)
      .await?;

    // TODO: check segment

    Ok(())
  }
}

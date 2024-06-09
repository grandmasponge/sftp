use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Replace the sample below with your own migration scripts
        manager
            .create_table(Table::create()
            .table(Sftp::Table)
            .if_not_exists()
            .col(
                ColumnDef::new(Sftp::Id)
                .integer()
                .not_null()
                .auto_increment()
                .primary_key()
            )
            .col(
                ColumnDef::new(Sftp::Username)
                .string()
                .unique_key()
                .not_null()
            )
            .col(
                ColumnDef::new(Sftp::Password)
                .string()
            )
            .col(
                ColumnDef::new(Sftp::Mountpoint)
                .string()
                .not_null()
            )
            .col(
                ColumnDef::new(Sftp::Keys)
                .json()
            ).to_owned()
         ).await
            
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
       
        manager
            .drop_table(Table::drop().table(Sftp::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum Sftp {
    Table,
    Id,
    Username,
    Password,
    Mountpoint,
    Keys
}

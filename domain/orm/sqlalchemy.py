from sqlalchemy.orm import DeclarativeBase

class Base(DeclarativeBase):
    pass

class Invoice(Base):
    __tablename__ = "invoices"

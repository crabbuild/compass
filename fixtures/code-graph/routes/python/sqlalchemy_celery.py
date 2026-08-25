from celery import shared_task
from sqlalchemy.orm import DeclarativeBase, Mapped, mapped_column, relationship


class Base(DeclarativeBase):
    pass


class User(Base):
    __tablename__ = "users"
    id: Mapped[int] = mapped_column(primary_key=True)


class Post(Base):
    __tablename__ = "posts"
    author: Mapped[User] = relationship(User)


@shared_task(queue="maintenance")
def refresh_users():
    return None


def dispatch_refresh():
    refresh_users.delay()

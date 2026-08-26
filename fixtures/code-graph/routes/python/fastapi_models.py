from fastapi import Depends, FastAPI, Security
from pydantic import BaseModel, field_validator


class Item(BaseModel):
    name: str

    @field_validator("name")
    @classmethod
    def valid_name(cls, value):
        return value


def database():
    yield object()


def authorize(database=Depends(database)):
    return True


app = FastAPI(dependencies=[Depends(database)])


@app.post("/items", dependencies=[Security(authorize)], response_model=Item)
def create_item(item: Item) -> Item:
    return item

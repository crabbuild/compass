from django.db import models

class Customer(models.Model):
    class Meta:
        db_table = "customers"

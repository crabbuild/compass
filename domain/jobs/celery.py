from celery import Celery, shared_task

app = Celery("tasks")

@app.task(name="orders.cleanup", queue="maintenance")
def cleanup_orders():
    pass

@shared_task
def refresh_inventory():
    pass

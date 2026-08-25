from django.contrib import admin
from django.db import models
from django.db.models.signals import post_save
from django.dispatch import receiver
from django.urls import include, path
from rest_framework.decorators import action
from rest_framework.permissions import IsAuthenticated
from rest_framework.routers import DefaultRouter
from rest_framework.serializers import ModelSerializer
from rest_framework.viewsets import ModelViewSet


class ItemManager(models.Manager):
    pass


class Owner(models.Model):
    name = models.CharField(max_length=100)


class Item(models.Model):
    slug = models.SlugField(unique=True)
    owner = models.ForeignKey(Owner, on_delete=models.CASCADE)
    objects = ItemManager()


class ItemSerializer(ModelSerializer):
    class Meta:
        model = Item
        fields = ["slug", "owner"]


class ItemViewSet(ModelViewSet):
    lookup_field = "slug"
    serializer_class = ItemSerializer
    permission_classes = [IsAuthenticated]

    def list(self, request):
        return None

    @action(detail=True, methods=["post"], url_path="publish")
    def publish(self, request, slug=None):
        return None


@receiver(post_save, sender=Item)
def item_saved(sender, instance, **kwargs):
    return None


router = DefaultRouter()
router.register("items", ItemViewSet, basename="item")
urlpatterns = [
    path("api/", include((router.urls, "api"), namespace="v1")),
]

# These exact-looking registrations remain source-only until the framework
# descriptor can advertise registration semantics without mislabeling Django
# as a bean container.
admin.site.register(Item)
MIDDLEWARE = ["project.middleware.RequestMiddleware"]

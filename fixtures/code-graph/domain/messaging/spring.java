import org.springframework.kafka.annotation.KafkaListener;
import org.springframework.amqp.rabbit.annotation.RabbitListener;
import org.springframework.context.event.EventListener;

class OrderEvents {
    @KafkaListener(topics = "orders.created")
    public void consume(String event) {}

    @RabbitListener(queues = "orders.queue")
    public void consumeQueue(String event) {}

    @EventListener(OrderCancelled.class)
    public void cancelled(OrderCancelled event) {}
}

class OrderCancelled {}

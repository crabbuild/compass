import { Controller } from '@nestjs/common';
import { EventPattern, MessagePattern } from '@nestjs/microservices';
import { SubscribeMessage } from '@nestjs/websockets';

@Controller()
export class OrdersConsumer {
  @MessagePattern('orders.created')
  handleCreated() {}

  @EventPattern('orders.cancelled')
  handleCancelled() {}

  @SubscribeMessage('orders.updated')
  handleSocketUpdate() {}

  publish() {
    this.client.emit('orders.published', {});
    this.client.send('orders.requested', {});
  }
}

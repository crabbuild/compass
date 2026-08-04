import { Controller, Get, Post } from "@nestjs/common";
import { EventPattern, MessagePattern } from "@nestjs/microservices";
import { Mutation, Query, Resolver } from "@nestjs/graphql";
import { SubscribeMessage, WebSocketGateway } from "@nestjs/websockets";

@Controller("/users")
export class UsersController {
  @Get(":userId")
  showUser() {}

  @Post()
  createUser() {}

  @MessagePattern("users.lookup")
  lookupUser() {}

  @EventPattern("users.created")
  userCreated() {}

}

@WebSocketGateway()
export class UsersGateway {
  @SubscribeMessage("users.watch")
  watchUsers() {}
}

@Resolver()
export class UsersResolver {
  @Query("user")
  user() {}

  @Mutation()
  createUser() {}
}

import { Controller, Get, Post } from "@nestjs/common";
import { EventPattern, MessagePattern } from "@nestjs/microservices";
import { Mutation, Query, Resolver } from "@nestjs/graphql";
import { SubscribeMessage } from "@nestjs/websockets";

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

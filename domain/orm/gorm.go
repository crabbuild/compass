package models

import "gorm.io/gorm"

type User struct {
	ID uint
}

func (User) TableName() string { return "users" }

var _ *gorm.DB

//
//  receiver.m
//  FlyingCarpetWithStoryboard
//
//  Created by Theron Spiegl on 8/25/17.
//  Copyright © 2017 Theron Spiegl. All rights reserved.
//

#import <Foundation/Foundation.h>
#include <sys/types.h>
#include <sys/stat.h>

char msg[10];

char *hey() {
    printf("Attempting to mkfifo\n");
    int res = mkfifo("/tmp/fc.fifo", 0666);
    if (res != 0) {
        printf("error making receiving fifo: %s\n", strerror(errno));
    } else {
        printf("fifo made\n");
    }
    int fd;
    fd = open("/tmp/fc.fifo", O_RDONLY);
    printf("fifo opened\n");
    read(fd, msg, 5);
    
    printf("fifo read\n");
    close(fd);
    printf("%s\n",msg);
    return msg;
}

char *ho() {
    char *sendmsg = "test";
    printf("Attempting to mkfifo (sending)\n");
    int res = mkfifo("/tmp/fc.fifo", 0666);
    if (res != 0) {
        printf("error making sending fifo: %s\n", strerror(errno));
    } else {
        printf("sending fifo made\n");
    }
    int fd;
    fd = open("/tmp/fc.fifo", O_WRONLY);
    printf("sending fifo opened\n");
    write(fd, sendmsg, 5);
    
    printf("fifo written, closing\n");
    close(fd);
    printf("%s\n",msg);
    return msg;
}
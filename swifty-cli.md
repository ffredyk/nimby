

<!-- Start of picture text -->
@QwiFty<br><!-- End of picture text -->



<!-- Start of picture text -->
CLI<br><!-- End of picture text -->

A QUICK GUIDE TO SWIFTY-CLI 

# Contents 

|Requirements ...................................................................................................................................... 3|
|---|
|Setup ................................................................................................................................................... 3|
|Writing a repository configuration ................................................................................................. 4|
|Repository configuration file ............................................................................................................. 5|
|repoName........................................................................................................................................ 5|
|basePath ............................................................................................................................................. 5|
|requiredMods, optionalMods............................................................................................................. 5|
|iconImagePath, repoImagePath ........................................................................................................ 5|
|clientParameters ................................................................................................................................ 6|
|Servers ................................................................................................................................................ 6|
|Name ............................................................................................................................................... 6|
|Address ........................................................................................................................................... 6|
|Port .................................................................................................................................................. 6|
|Password ........................................................................................................................................ 6|
|battleEye .......................................................................................................................................... 6|



## Requirements 

To host a Swifty repository, you must be able to host your content on a server capable of serving content over HTTP. 

Sometimes they are referred to as a webserver so any server capable of hosting a website can host a Swifty repository. 

Note, that it may be against certain providers TOS to host a repository, consult your providers documentation before. 

There are many kinds of webservers but the most common are the following. 

- Apache 

- Nginx 

- IIS (windows) 

## Setup 

Swifty repositories are composed of Arma 3 addons. 

Arma 3 addon folders require a specific naming scheme with a starting ‘@’ before the modName. 

A repo must contain one or more addons and can contain optional addons. 

An easy setup is shown below with a main folder for the require mods and an optional folder. 

#### **repo** 

#### **addons** 

**@cba** 

#### **optionals** 

**@jsrs** 

Bolded names are folders. 

### Writing a repository configuration 

The repository configuration is used with the swifty-cli to configure various options. The configuration file is a json file that can be automatically generated using the swifty-cli. Windows 

#### <mark>swifty-cli.exe new</mark> 

Linux 

#### <mark>Dotnet swifty-cli.dll new</mark> 

Follow instructions and complete the wizard, you will now have a base repo configuration file. 

## Repository configuration file 

The repository configuration is stored in a JSON file. 

The following text lists each attribute in turn and what it does. 

### repoName 

The name of the repo to display in the client 

### basePath 

Sets the base path where swifty-cli will look for files. 

requiredMods, optionalMods 

An array of mods 

<mark>{</mark> “modName”: “” } 

modName is the name of a folder or a basic wildcard formula like optionals/@* 

iconImagePath, repoImagePath 

Path to image for icon and repo image. 

Icon should be a 32x32 for best effect, repo image should be around 400x100 

### clientParameters 

Default parameters set on the client upon import. 

Servers 

An array of server config. 

### Name 

Name of the server 

### Address 

Ip or address to server 

### Port 

Arma 3 server gameport 

Password 

Password to server 

### battleEye 

To launch with battleeye or not. 


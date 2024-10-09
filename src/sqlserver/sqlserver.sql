-- Create a new database called 'DatabaseName'
-- Connect to the 'master' database to run this snippet
USE master
GO
-- Create the new database if it does not exist already
IF NOT EXISTS (
    SELECT name
FROM sys.databases
WHERE name = N'MyLittleProxy'
)
CREATE DATABASE MyLittleProxy
GO

-- Switch context to the new database
USE MyLittleProxy
GO

-- Drop the table if it already exists
IF OBJECT_ID('blah', 'U') IS NOT NULL
DROP TABLE blah
GO

-- Create the table in the specified schema
CREATE TABLE blah
(
    id BIGINT IDENTITY(1,1) PRIMARY KEY,
    -- primary key column with identity
    t NVARCHAR(MAX),
    -- text column
    j NVARCHAR(MAX),
    -- JSON column
    vtha NVARCHAR(MAX)
    -- JSON column
);
GO